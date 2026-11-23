//! What happens when somebody edits the database file while a run waits for a person: an approval is for one exact call, and an
//! altered call is never run on the strength of an answer given to the original.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use zorix_core::{
    ApprovalDecision, ExecutionEngine, ExecutionRepository, ExecutionState as S, Grants, NewExecution, Permission,
    TaskRequest, TenantId, Tool, ToolAnnotations, ToolContext, ToolError, ToolRegistry, ToolSpec,
};
use zorix_runtime::{Engine, EngineConfig, InMemoryToolRegistry, RetryPolicy};
use zorix_sim::{ScriptedModel, Turn};
use zorix_store::SqliteRepository;

fn local() -> TenantId {
    TenantId::local()
}

struct Writer {
    spec: ToolSpec,
    written: parking_lot::Mutex<Vec<String>>,
}

impl Writer {
    fn new() -> Arc<Self> {
        let schema = json!({"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]});
        let mut spec = ToolSpec::new("files.write", "Write", "writes a file", schema);
        spec.annotations =
            ToolAnnotations { read_only: false, destructive: true, idempotent: false, open_world: false };
        Arc::new(Self { spec, written: Default::default() })
    }

    fn paths(&self) -> Vec<String> {
        self.written.lock().clone()
    }
}

#[async_trait]
impl Tool for Writer {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, _ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        self.written.lock().push(args["path"].as_str().unwrap_or_default().to_string());
        Ok(json!({"written": args["path"]}))
    }
}

struct Rig {
    engine: Engine,
    writer: Arc<Writer>,
    path: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zorix.sqlite");
    let tools = Arc::new(InMemoryToolRegistry::new());
    let writer = Writer::new();
    tools.register(writer.clone()).unwrap();
    let config = EngineConfig {
        workers: 2,
        lease_ttl_ms: 1_000,
        retry: RetryPolicy { base_delay_ms: 1, max_delay_ms: 5 },
        sole_owner: true,
        ..EngineConfig::default()
    };
    let model = ScriptedModel::new(vec![
        Turn::call("files.write", json!({"path": "a.txt"})),
        Turn::Say("saved".into()),
        Turn::Say("saved".into()),
    ]);
    let engine = Engine::builder(Arc::new(SqliteRepository::open(&path).unwrap()), Arc::new(model))
        .tools(tools)
        .config(config)
        .build();
    Rig { engine, writer, path, _dir: dir }
}

async fn waiting(rig: &Rig) -> zorix_core::Execution {
    let grants = Grants::none().with(Permission::use_tool("files.write"));
    let request = NewExecution {
        request: TaskRequest { prompt: "save a.txt".into(), model: "sim".into(), ..TaskRequest::default() },
        grants,
        ..NewExecution::default()
    };
    let run = rig.engine.submit(request).await.unwrap();
    let run =
        tokio::time::timeout(Duration::from_secs(10), rig.engine.settle(&local(), run.id)).await.unwrap().unwrap();
    assert_eq!(run.state, S::WaitingForApproval);
    run
}

fn yes(hash: &str) -> ApprovalDecision {
    ApprovalDecision {
        approve: true,
        decided_by: "tester".into(),
        note: None,
        call_hash: Some(hash.into()),
        automatic: false,
    }
}

#[tokio::test]
async fn a_call_edited_on_disk_while_it_waits_is_not_run_on_the_answer_given_to_the_original() {
    let rig = rig();
    let run = waiting(&rig).await;
    let reader = SqliteRepository::open(&rig.path).unwrap();
    let approval = reader.approvals(run.id).await.unwrap().remove(0);

    // Somebody rewrites the pending call in the checkpoint and the tool-call record: a.txt becomes something else.
    let conn = rusqlite::Connection::open(&rig.path).unwrap();
    let changed = conn.execute("UPDATE checkpoints SET doc = replace(doc, 'a.txt', '../../evil.txt')", []).unwrap()
        + conn.execute("UPDATE tool_calls SET doc = replace(doc, 'a.txt', '../../evil.txt')", []).unwrap();
    assert!(changed >= 1, "the test must really have edited something");

    // The person says yes to what they were shown (a.txt).
    rig.engine.decide(&local(), run.id, approval.id, yes(&approval.call_hash)).await.unwrap();
    let after =
        tokio::time::timeout(Duration::from_secs(10), rig.engine.settle(&local(), run.id)).await.unwrap().unwrap();

    assert!(rig.writer.paths().is_empty(), "the altered call never ran: {:?}", rig.writer.paths());
    assert_eq!(after.state, S::WaitingForApproval, "the altered call asks again instead of running");
    let approvals = reader.approvals(run.id).await.unwrap();
    assert_eq!(approvals.len(), 2, "the old answer stays what it was, and the new call has its own approval");
    assert_eq!(approvals[0].arguments["path"], "a.txt");
    assert_eq!(approvals[1].arguments["path"], "../../evil.txt");
    assert_ne!(approvals[1].call_hash, approval.call_hash);
    assert_eq!(approvals[1].status, zorix_core::ApprovalStatus::Pending);
}

#[tokio::test]
async fn an_approval_edited_on_disk_cannot_be_answered_with_the_hash_the_person_saw() {
    let rig = rig();
    let run = waiting(&rig).await;
    let reader = SqliteRepository::open(&rig.path).unwrap();
    let approval = reader.approvals(run.id).await.unwrap().remove(0);

    let conn = rusqlite::Connection::open(&rig.path).unwrap();
    conn.execute("UPDATE approvals SET doc = replace(doc, ?1, 'feedface')", [&approval.call_hash]).unwrap();

    let result = rig.engine.decide(&local(), run.id, approval.id, yes(&approval.call_hash)).await;
    assert!(matches!(result, Err(zorix_core::Error::InvalidInput(_))), "{result:?}");
    assert!(rig.writer.paths().is_empty());
}
