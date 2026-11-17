//! Operational memory through a real engine: every finished execution is recorded without slowing it, can be found later,
//! and is kept apart by tenant and project.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use zorix_core::{
    ApprovalDecision, ApprovalStatus, ConversationId, Corpus, Execution, ExecutionEngine, ExecutionId,
    ExecutionState as S, Grants, MemoryStore, NewExecution, OperationRecord, Permission, TaskRequest, TenantId,
    Timestamp, Visibility,
};
use zorix_memory::{MemoryStack, OperationRecorder};
use zorix_runtime::{Engine, EngineConfig, InMemoryToolRegistry};
use zorix_sim::{ScriptedModel, Turn};
use zorix_store::{SqliteMemoryStore, SqliteRepository};

struct Env {
    _dir: tempfile::TempDir,
    repository: Arc<SqliteRepository>,
    store: Arc<SqliteMemoryStore>,
    engine: Engine,
    model: Arc<ScriptedModel>,
}

fn env(turns: Vec<Turn>) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zorix.sqlite");
    let repository = Arc::new(SqliteRepository::open(&path).unwrap());
    let store = Arc::new(SqliteMemoryStore::open(&path).unwrap());
    let stack = MemoryStack::new(store.clone(), repository.clone(), None);
    let registry = Arc::new(InMemoryToolRegistry::new());
    stack.register_tools(&*registry).unwrap();
    zorix_tools::register_builtin(
        &*registry,
        &zorix_tools::BuiltinConfig { notes_file: dir.path().join("notes.json") },
    )
    .unwrap();
    let model = Arc::new(ScriptedModel::new(turns));
    let engine = stack
        .install(Engine::builder(repository.clone(), model.clone()).tools(registry))
        .config(EngineConfig::default())
        .build();
    Env { _dir: dir, repository, store, engine, model }
}

/// An absolute path that exists on no machine and is valid on every system ("/work/one" is not absolute on Windows).
fn project_path(name: &str) -> String {
    std::env::temp_dir().join("zorix-work").join(name).to_string_lossy().into_owned()
}

fn new_run(prompt: &str, workspace: Option<&str>, conversation: Option<&str>) -> NewExecution {
    NewExecution {
        conversation: conversation.map(|c| ConversationId::new(c).unwrap()),
        request: TaskRequest { prompt: prompt.into(), model: "sim".into(), ..TaskRequest::default() },
        grants: Grants::none()
            .with(Permission::use_tool("math.evaluate"))
            .with(Permission::use_tool("history.search"))
            .with(Permission::use_tool("notes.put")),
        workspace: workspace.map(Into::into),
        ..NewExecution::default()
    }
}

async fn run(env: &Env, new: NewExecution) -> Execution {
    let started = env.engine.submit(new).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), env.engine.settle(&TenantId::local(), started.id))
        .await
        .unwrap()
        .unwrap()
}

/// Wait for the background task to write a record.
async fn recorded(env: &Env, id: ExecutionId) -> OperationRecord {
    for _ in 0..200 {
        if let Some(record) = env.store.operation(&TenantId::local(), id).await.unwrap() {
            return record;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the operation of {id} was never recorded");
}

fn vis(project: Option<&str>) -> Visibility {
    Visibility { tenant: TenantId::local(), project: project.map(str::to_string), conversation: None }
}

#[tokio::test]
async fn a_finished_execution_is_recorded_with_its_tools_and_found_by_later_work() {
    let env = env(vec![
        Turn::call("math.evaluate", json!({"expression": "17*23"})),
        Turn::Say("It is 391.".into()),
        Turn::Say("Same as before: 391.".into()),
    ]);
    let first = run(
        &env,
        new_run(
            "Calculate seventeen times twenty three for the invoice total",
            Some(project_path("one").as_str()),
            Some("chat-a"),
        ),
    )
    .await;
    assert_eq!(first.state, S::Completed);
    let record = recorded(&env, first.id).await;
    assert_eq!((record.state.as_str(), record.project.as_deref()), ("completed", Some(project_path("one").as_str())));
    assert!(
        record.text.contains("invoice total")
            && record.text.contains("It is 391.")
            && record.text.contains("math.evaluate x1"),
        "{}",
        record.text
    );
    // A later execution of the same project, in another conversation, is told about it as past work.
    let second = run(
        &env,
        new_run("What was the invoice total calculation?", Some(project_path("one").as_str()), Some("chat-b")),
    )
    .await;
    assert_eq!(second.state, S::Completed);
    let seen = &env.model.requests()[2];
    let work = seen.messages.iter().find(|m| m.content.starts_with("<past_work")).expect("past work in the context");
    assert!(
        work.content.contains(&first.id.to_string())
            && work.content.contains("391")
            && work.content.contains("untrusted=\"true\"")
    );
    // Another project does not see it.
    assert!(
        env.store
            .text_search(&vis(Some(project_path("two").as_str())), Corpus::Operations, &["invoice".into()], 5)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn failed_and_cancelled_executions_are_recorded_too_and_approvals_are_part_of_the_story() {
    let env = env(vec![
        Turn::Fail(zorix_sim::Failure::Permanent("the model refused".into())),
        Turn::call("notes.put", json!({"key": "plan", "value": "x"})),
        Turn::Say("ok, not saved".into()),
    ]);
    let failed = run(&env, new_run("An impossible request about the archive", None, None)).await;
    assert_eq!(failed.state, S::Failed);
    let record = recorded(&env, failed.id).await;
    assert_eq!(record.state, "failed");
    assert!(record.text.contains("the model refused"));

    let waiting = run(&env, new_run("Save the plan about the archive as a note", None, None)).await;
    assert_eq!(waiting.state, S::WaitingForApproval);
    let detail = env.engine.detail(&TenantId::local(), waiting.id).await.unwrap();
    let approval = detail.approvals.iter().find(|a| a.status == ApprovalStatus::Pending).unwrap().clone();
    let no = ApprovalDecision {
        approve: false,
        decided_by: "luis".into(),
        note: Some("not yet".into()),
        call_hash: Some(approval.call_hash),
        automatic: false,
    };
    env.engine.decide(&TenantId::local(), waiting.id, approval.id, no).await.unwrap();
    let done = tokio::time::timeout(Duration::from_secs(10), env.engine.settle(&TenantId::local(), waiting.id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.state, S::Completed);
    let record = recorded(&env, waiting.id).await;
    assert!(record.text.contains("Approval: notes.put denied by luis (not yet)"), "{}", record.text);
}

#[tokio::test]
async fn executions_from_before_the_recorder_existed_are_recorded_by_the_backfill() {
    // Run with an engine that has no memory at all...
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zorix.sqlite");
    let repository = Arc::new(SqliteRepository::open(&path).unwrap());
    let model = Arc::new(ScriptedModel::new([Turn::Say("old answer one".into()), Turn::Say("old answer two".into())]));
    let engine = Engine::builder(repository.clone(), model).config(EngineConfig::default()).build();
    let mut ids = Vec::new();
    for prompt in ["Old question one about invoices", "Old question two about invoices"] {
        let started = engine.submit(new_run(prompt, None, None)).await.unwrap();
        tokio::time::timeout(Duration::from_secs(10), engine.settle(&TenantId::local(), started.id))
            .await
            .unwrap()
            .unwrap();
        ids.push(started.id);
    }
    engine.shutdown().await;
    // ...then start memory over the same database.
    let store = Arc::new(SqliteMemoryStore::open(&path).unwrap());
    assert!(store.operation(&TenantId::local(), ids[0]).await.unwrap().is_none());
    let stack = MemoryStack::new(store.clone(), repository.clone(), None);
    let _engine = stack.install(Engine::builder(repository, Arc::new(ScriptedModel::new([]))));
    assert_eq!(stack.backfill_operations().await.unwrap(), 2);
    for id in ids {
        assert!(store.operation(&TenantId::local(), id).await.unwrap().is_some());
    }
    assert_eq!(stack.backfill_operations().await.unwrap(), 0, "nothing left to do");
    assert_eq!(store.text_search(&vis(None), Corpus::Operations, &["invoices".into()], 5).await.unwrap().len(), 2);
}

#[tokio::test]
async fn a_full_queue_never_blocks_the_engine_and_the_backfill_catches_up() {
    let env = env(vec![Turn::Say("a".into()), Turn::Say("b".into()), Turn::Say("c".into()), Turn::Say("d".into())]);
    // A recorder whose queue holds a single record: publishing many events returns at once.
    let tiny = OperationRecorder::start_with_queue(env.store.clone(), env.repository.clone(), None, 1);
    let ids: Vec<ExecutionId> = {
        let mut ids = Vec::new();
        for prompt in ["one", "two", "three", "four"] {
            ids.push(run(&env, new_run(prompt, None, None)).await.id);
        }
        ids
    };
    let events = {
        use zorix_core::EventPublisher;
        let events: Vec<zorix_core::ExecutionEvent> = ids
            .iter()
            .map(|id| zorix_core::ExecutionEvent {
                seq: 1,
                execution_id: *id,
                at: Timestamp(1),
                schema_version: 1,
                kind: zorix_core::EventKind::Completed,
                payload: json!({}),
            })
            .collect();
        let started = std::time::Instant::now();
        for _ in 0..1_000 {
            tiny.publish(&events);
        }
        assert!(started.elapsed() < Duration::from_secs(2), "publishing never waits");
        events
    };
    assert_eq!(events.len(), 4);
    for id in &ids {
        recorded(&env, *id).await;
    }
    // Whatever was dropped is found by the backfill.
    assert_eq!(tiny.backfill(10).await.unwrap(), 0);
}

#[tokio::test]
async fn history_search_returns_past_work_of_this_project_only() {
    let env = env(vec![
        Turn::Say("Rotated the staging credentials.".into()),
        Turn::Say("Updated the production dashboards.".into()),
        Turn::call("history.search", json!({"query": "staging credentials"})),
        Turn::Say("found it".into()),
    ]);
    let a =
        run(&env, new_run("Rotate the staging credentials for the deploy", Some(project_path("one").as_str()), None))
            .await;
    recorded(&env, a.id).await;
    let b =
        run(&env, new_run("Update the production dashboards for the deploy", Some(project_path("two").as_str()), None))
            .await;
    recorded(&env, b.id).await;
    let asking =
        run(&env, new_run("Did we rotate the staging credentials?", Some(project_path("one").as_str()), None)).await;
    assert_eq!(asking.state, S::Completed);
    let result = env
        .model
        .requests()
        .iter()
        .flat_map(|r| r.messages.clone())
        .filter(|m| m.content.starts_with("<tool_result"))
        .map(|m| m.content)
        .next_back()
        .expect("the tool result");
    assert!(result.contains("Rotated the staging credentials") && result.contains(&a.id.to_string()), "{result}");
    assert!(!result.contains("production dashboards"), "the other project's work is not shown");
    // It is a read: no approval was needed for it.
    let detail = env.engine.detail(&TenantId::local(), asking.id).await.unwrap();
    assert!(detail.approvals.is_empty());
}

#[tokio::test]
async fn another_tenants_work_is_never_found() {
    let env = env(vec![Turn::Say("ok".into())]);
    let other = TenantId::new("other").unwrap();
    env.store
        .record_operation(OperationRecord {
            execution_id: ExecutionId::new(),
            tenant: other,
            project: None,
            conversation: None,
            state: "completed".into(),
            finished_at: Timestamp(1),
            text: "The other tenant planned a confidential acquisition.".into(),
            doc: json!({}),
        })
        .await
        .unwrap();
    let run = run(&env, new_run("Tell me about the confidential acquisition plan", None, None)).await;
    assert_eq!(run.state, S::Completed);
    assert!(
        !env.model.requests()[0]
            .messages
            .iter()
            .any(|m| m.content.contains("acquisition plan") && m.content.starts_with("<past_work"))
    );
}
