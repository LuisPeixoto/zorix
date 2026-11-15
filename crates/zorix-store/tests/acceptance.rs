//! The acceptance scenario of Phase 1, step by step, on a real database file and with the real built-in tools.
//!
//! > A client sends an activity. The runtime creates an execution, builds the context, asks the model, which asks for a
//! > tool. The runtime checks permissions, validates the parameters, runs the tool, feeds the result back, and goes on
//! > until the model gives a final answer. Events are published and state is persisted all along. When a tool needs
//! > approval the execution is suspended and later resumed. A failure is recorded and recovered. The result is
//! > available to the client. All of it with a simulated model.

use std::sync::Arc;
use std::time::Duration;

use zorix_core::{
    ApprovalDecision, ExecutionEngine, ExecutionState as S, Grants, NewExecution, Permission, TaskRequest, TenantId,
};
use zorix_runtime::{Engine, EngineConfig, InMemoryToolRegistry};
use zorix_sim::DemoModel;
use zorix_store::SqliteRepository;
use zorix_tools::{BuiltinConfig, register_builtin};

fn engine(dir: &std::path::Path) -> Engine {
    let registry = Arc::new(InMemoryToolRegistry::new());
    register_builtin(&*registry, &BuiltinConfig { notes_file: dir.join("notes.json") }).unwrap();
    let repository = Arc::new(SqliteRepository::open(&dir.join("zorix.sqlite")).unwrap());
    let config = EngineConfig { sole_owner: true, lease_ttl_ms: 1_000, ..EngineConfig::default() };
    Engine::builder(repository, Arc::new(DemoModel::new())).tools(registry).config(config).build()
}

async fn settle(engine: &Engine, id: zorix_core::ExecutionId) -> zorix_core::Execution {
    tokio::time::timeout(Duration::from_secs(10), engine.settle(&TenantId::local(), id)).await.unwrap().unwrap()
}

#[tokio::test]
async fn the_scenario_from_the_brief() {
    let dir = tempfile::tempdir().unwrap();
    let tenant = TenantId::local();

    // 1. A client sends an activity.
    let first = engine(dir.path());
    let grants = Grants::none().with(Permission::use_tool("math.evaluate")).with(Permission::use_tool("notes.put"));
    let new = NewExecution {
        request: TaskRequest {
            prompt: "What is 17*23 and save it as result".into(),
            model: "demo".into(),
            ..TaskRequest::default()
        },
        grants,
        ..NewExecution::default()
    };
    let created = first.create(new).await.unwrap();
    assert_eq!(created.state, S::Created, "the runtime created an execution");

    // Events are published while it runs.
    let live = first.subscribe(&tenant, created.id, 0).await.unwrap();
    first.start(&tenant, created.id).await.unwrap();

    // 2. The model asked for a tool; permissions were checked, parameters validated, the tool ran, its result went back
    //    to the model, which asked for a second tool. That one needs a person.
    let waiting = settle(&first, created.id).await;
    assert_eq!(waiting.state, S::WaitingForApproval);
    assert_eq!(waiting.usage.tool_calls, 1, "math.evaluate ran");
    let detail = first.detail(&tenant, created.id).await.unwrap();
    assert_eq!(detail.tool_calls[0].result.as_ref().unwrap()["result"], 391);
    let approval = detail.approvals.iter().find(|a| a.tool == "notes.put").expect("approval for notes.put").clone();
    assert!(!dir.path().join("notes.json").exists(), "the second tool has not run");

    // 3. State is persisted: the process goes away while the execution waits.
    first.shutdown().await;
    drop(live);
    drop(first);

    // 4. A new process finds it waiting, a person approves, and it resumes where it stopped.
    let second = engine(dir.path());
    let report = second.recover().await.unwrap();
    assert!(report.requeued.is_empty(), "nothing is re-run: it is waiting for a person");
    let decision = ApprovalDecision {
        approve: true,
        decided_by: "reviewer".into(),
        note: None,
        call_hash: Some(approval.call_hash.clone()),
    };
    second.decide(&tenant, created.id, approval.id, decision).await.unwrap();
    let done = settle(&second, created.id).await;

    // 5. The model produced a final answer; the result is available to the client.
    assert_eq!(done.state, S::Completed);
    let answer = done.result.unwrap().text;
    assert!(answer.contains("391") && answer.contains("notes.put"), "{answer}");
    assert_eq!(std::fs::read_to_string(dir.path().join("notes.json")).unwrap().matches("17*23 = 391").count(), 1);

    // 6. The whole story is recorded, in order, without gaps.
    let events = second.events(&tenant, created.id, 0).await.unwrap();
    let names: Vec<&str> = events.iter().map(|e| e.kind.name()).collect();
    let expected = [
        "execution.created",
        "execution.queued",
        "execution.started",
        "execution.model.requested",
        "execution.model.responded",
        "execution.tool.requested",
        "execution.tool.started",
        "execution.tool.completed",
        "execution.model.requested",
        "execution.model.responded",
        "execution.tool.requested",
        "execution.waiting_approval",
        "execution.approval.granted",
        "execution.tool.started",
        "execution.tool.completed",
        "execution.model.requested",
        "execution.model.responded",
        "execution.completed",
    ];
    let mut at = 0;
    for name in &names {
        if at < expected.len() && *name == expected[at] {
            at += 1;
        }
    }
    assert_eq!(at, expected.len(), "events out of order: {names:?}");
    assert!(events.windows(2).all(|w| w[0].seq < w[1].seq));

    // And there is a summary of what ran, with what, how long, and at what cost.
    let summary = second.summary(&tenant, created.id).await.unwrap();
    assert_eq!((summary.model_calls, summary.tool_calls.len()), (3, 2));
    assert!(summary.usage.total_tokens() > 0 && summary.total_ms.is_some());
}

#[tokio::test]
async fn and_when_a_tool_fails_the_failure_is_recorded_and_the_model_can_recover() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    let new = NewExecution {
        request: TaskRequest {
            prompt: "What is 1/0 and save it as nothing".into(),
            model: "demo".into(),
            ..TaskRequest::default()
        },
        grants: Grants::none().with(Permission::use_tool("math.evaluate")).with(Permission::use_tool("notes.put")),
        ..NewExecution::default()
    };
    let started = engine.submit(new).await.unwrap();
    let done = settle(&engine, started.id).await;
    assert_eq!(done.state, S::Completed, "the model reported the problem instead of crashing the run");
    assert!(done.result.unwrap().text.contains("Division by zero."));
    let detail = engine.detail(&TenantId::local(), started.id).await.unwrap();
    assert_eq!(detail.tool_calls[0].error.as_deref(), Some("Division by zero."));
    assert!(!dir.path().join("notes.json").exists(), "and the second tool was never asked for");
}
