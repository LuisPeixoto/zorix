//! What only a real file can show: data survives closing, two connections cannot both win, and the engine recovers from
//! a "crash" using nothing but what is on disk.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use zorix_core::{
    ApprovalDecision, Commit, Error, Execution, ExecutionEngine, ExecutionRepository, ExecutionState as S, Grants,
    NewExecution, Permission, TaskRequest, TenantId, Tool, ToolAnnotations, ToolContext, ToolError, ToolRegistry,
    ToolSpec, Trigger,
};
use zorix_runtime::testing::{event, sample_execution};
use zorix_runtime::{Engine, EngineConfig, InMemoryToolRegistry, RetryPolicy};
use zorix_sim::{ScriptedModel, Turn};
use zorix_store::{SCHEMA_VERSION, SqliteRepository};

fn config() -> EngineConfig {
    EngineConfig {
        workers: 2,
        lease_ttl_ms: 1_000,
        retry: RetryPolicy { base_delay_ms: 1, max_delay_ms: 5 },
        sole_owner: true,
        ..EngineConfig::default()
    }
}

fn local() -> TenantId {
    TenantId::local()
}

struct Writer {
    spec: ToolSpec,
    runs: std::sync::atomic::AtomicU32,
}

impl Writer {
    fn new() -> Arc<Self> {
        let schema = json!({"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]});
        let mut spec = ToolSpec::new("files.write", "Write", "writes a file", schema);
        spec.annotations =
            ToolAnnotations { read_only: false, destructive: true, idempotent: false, open_world: false };
        Arc::new(Self { spec, runs: Default::default() })
    }

    fn count(&self) -> u32 {
        self.runs.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait]
impl Tool for Writer {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, _ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        self.runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(json!({"written": args["path"]}))
    }
}

fn engine(repo: SqliteRepository, turns: Vec<Turn>, tools: Arc<InMemoryToolRegistry>) -> Engine {
    Engine::builder(Arc::new(repo), Arc::new(ScriptedModel::new(turns))).tools(tools).config(config()).build()
}

fn request(prompt: &str, grants: Grants) -> NewExecution {
    NewExecution {
        request: TaskRequest { prompt: prompt.into(), model: "sim".into(), ..TaskRequest::default() },
        grants,
        ..NewExecution::default()
    }
}

#[tokio::test]
async fn everything_survives_closing_and_reopening_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested/zorix.sqlite");
    let id = {
        let repo = SqliteRepository::open(&path).unwrap();
        let execution = sample_execution("acme", Some("key-1"));
        let id = execution.id;
        repo.create(execution, vec![event(zorix_core::EventKind::Created)]).await.unwrap();
        id
    };
    let repo = SqliteRepository::open(&path).unwrap();
    let tenant = TenantId::new("acme").unwrap();
    let found = repo.get(&tenant, id).await.unwrap().expect("still there");
    assert_eq!((found.version, found.idempotency_key.as_deref()), (1, Some("key-1")));
    assert_eq!(repo.events(id, 0, 10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn the_file_uses_the_write_ahead_log_and_the_current_schema() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zorix.sqlite");
    SqliteRepository::open(&path).unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    let mode: String = conn.pragma_query_value(None, "journal_mode", |r| r.get(0)).unwrap();
    let version: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
    assert_eq!((mode.as_str(), version), ("wal", SCHEMA_VERSION));
}

#[tokio::test]
async fn two_connections_cannot_both_apply_a_change_made_from_the_same_version() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zorix.sqlite");
    let first = SqliteRepository::open(&path).unwrap();
    let second = SqliteRepository::open(&path).unwrap();
    let mut execution = sample_execution("acme", None);
    first.create(execution.clone(), vec![]).await.unwrap();
    execution.apply(Trigger::Enqueue, zorix_core::Timestamp(5)).unwrap();

    let a = first.commit(Commit::new(execution.clone(), 1));
    let b = second.commit(Commit::new(execution.clone(), 1));
    let (a, b) = tokio::join!(a, b);
    let results = [a.map(|_| ()), b.map(|_| ())];
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1, "exactly one wins: {results:?}");
    assert!(results.iter().any(|r| matches!(r, Err(Error::Conflict(_)))));
}

#[tokio::test]
async fn a_failed_commit_leaves_no_half_written_state() {
    let repo = SqliteRepository::open_in_memory().unwrap();
    let mut execution = sample_execution("acme", None);
    repo.create(execution.clone(), vec![event(zorix_core::EventKind::Created)]).await.unwrap();
    execution.apply(Trigger::Enqueue, zorix_core::Timestamp(5)).unwrap();
    let mut stale = Commit::new(execution.clone(), 99);
    stale.events = vec![event(zorix_core::EventKind::Queued)];
    assert!(repo.commit(stale).await.is_err());
    assert_eq!(repo.events(execution.id, 0, 10).await.unwrap().len(), 1);
    let tenant = TenantId::new("acme").unwrap();
    assert_eq!(repo.get(&tenant, execution.id).await.unwrap().unwrap().state, S::Created);
}

#[tokio::test]
async fn a_damaged_record_is_reported_instead_of_crashing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zorix.sqlite");
    let repo = SqliteRepository::open(&path).unwrap();
    let execution: Execution = sample_execution("acme", None);
    repo.create(execution.clone(), vec![]).await.unwrap();
    rusqlite::Connection::open(&path).unwrap().execute("UPDATE executions SET doc = '{not json'", []).unwrap();
    let error = repo.load(execution.id).await.unwrap_err();
    assert!(matches!(&error, Error::Storage(text) if text.contains("could not be read")), "{error}");
}

#[tokio::test]
async fn a_whole_run_with_an_approval_is_stored_and_can_be_read_back_by_another_connection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zorix.sqlite");
    let tools = Arc::new(InMemoryToolRegistry::new());
    let writer = Writer::new();
    tools.register(writer.clone()).unwrap();
    let engine = engine(
        SqliteRepository::open(&path).unwrap(),
        vec![Turn::call("files.write", json!({"path": "a.txt"})), Turn::Say("saved".into())],
        tools,
    );
    let grants = Grants::none().with(Permission::use_tool("files.write"));
    let waiting = engine.submit(request("save a.txt", grants)).await.unwrap();
    let waiting =
        tokio::time::timeout(Duration::from_secs(10), engine.settle(&local(), waiting.id)).await.unwrap().unwrap();
    assert_eq!(waiting.state, S::WaitingForApproval);

    // Another connection (the CLI looking around) sees the pending approval on disk.
    let reader = SqliteRepository::open(&path).unwrap();
    let approvals = reader.approvals(waiting.id).await.unwrap();
    assert_eq!(approvals.len(), 1);
    let calls = reader.tool_calls(waiting.id).await.unwrap();
    assert_eq!(calls[0].tool, "files.write");

    let decision = ApprovalDecision {
        approve: true,
        decided_by: "tester".into(),
        note: None,
        call_hash: Some(approvals[0].call_hash.clone()),
        automatic: false,
    };
    engine.decide(&local(), waiting.id, approvals[0].id, decision).await.unwrap();
    let done =
        tokio::time::timeout(Duration::from_secs(10), engine.settle(&local(), waiting.id)).await.unwrap().unwrap();
    assert_eq!(done.state, S::Completed);
    assert_eq!(writer.count(), 1);
    let stored = reader.events(waiting.id, 0, 1000).await.unwrap();
    assert_eq!(stored.last().map(|e| e.kind.name()), Some("execution.completed"));
    assert!(stored.windows(2).all(|w| w[0].seq < w[1].seq));
}

#[tokio::test]
async fn after_a_crash_a_new_process_continues_from_what_is_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zorix.sqlite");

    let first = engine(SqliteRepository::open(&path).unwrap(), vec![Turn::Hang], Arc::new(InMemoryToolRegistry::new()));
    let started = first.submit(request("go", Grants::none())).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while first.get(&local(), started.id).await.unwrap().state != S::WaitingForModel {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    first.abort();
    drop(first);

    // A brand new connection and engine, as after a restart.
    let second = engine(
        SqliteRepository::open(&path).unwrap(),
        vec![Turn::Say("back again".into())],
        Arc::new(InMemoryToolRegistry::new()),
    );
    let report = second.recover().await.unwrap();
    assert_eq!(report.requeued, vec![started.id]);
    let done =
        tokio::time::timeout(Duration::from_secs(10), second.settle(&local(), started.id)).await.unwrap().unwrap();
    assert_eq!(done.state, S::Completed);
    assert_eq!(done.result.unwrap().text, "back again");
    let kinds: Vec<&str> = second
        .events(&local(), started.id, 0)
        .await
        .unwrap()
        .iter()
        .map(|e| e.kind.name())
        .collect::<Vec<_>>()
        .into_iter()
        .collect();
    assert!(kinds.contains(&"execution.recovered") && kinds.last() == Some(&"execution.completed"), "{kinds:?}");
}

#[tokio::test]
async fn the_idempotency_key_holds_across_connections() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zorix.sqlite");
    let a = SqliteRepository::open(&path).unwrap();
    let b = SqliteRepository::open(&path).unwrap();
    let (first, second) = (sample_execution("acme", Some("k")), sample_execution("acme", Some("k")));
    let (x, y) = tokio::join!(a.create(first, vec![]), b.create(second, vec![]));
    let created = [&x, &y].iter().filter(|r| matches!(r, Ok(zorix_core::CreateOutcome::Created(_)))).count();
    let existing = [&x, &y].iter().filter(|r| matches!(r, Ok(zorix_core::CreateOutcome::Existing(_)))).count();
    assert_eq!((created, existing), (1, 1), "{x:?} {y:?}");
    let tenant = TenantId::new("acme").unwrap();
    assert_eq!(a.list(&tenant, &Default::default()).await.unwrap().len(), 1);
}
