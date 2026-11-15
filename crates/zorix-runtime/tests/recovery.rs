//! Crashes, restarts and graceful shutdown: executions continue from their last checkpoint.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use serde_json::json;
use zorix_core::{ExecutionEngine, ExecutionState as S, ToolCallStatus};
use zorix_runtime::{Engine, EngineConfig, InMemoryToolRegistry, MemoryRepository};
use zorix_sim::{ScriptedModel, Turn};

/// A second engine on the same store, as after a restart.
fn restarted(
    repo: &Arc<MemoryRepository>,
    turns: Vec<Turn>,
    tools: Arc<InMemoryToolRegistry>,
    sole_owner: bool,
) -> (Engine, Arc<ScriptedModel>) {
    let model = Arc::new(ScriptedModel::new(turns));
    let config = EngineConfig { sole_owner, ..fast_config() };
    (Engine::builder(repo.clone(), model.clone()).tools(tools).config(config).build(), model)
}

fn registry_of(tools: Vec<Arc<Counting>>) -> Arc<InMemoryToolRegistry> {
    use zorix_core::ToolRegistry;
    let registry = Arc::new(InMemoryToolRegistry::new());
    for tool in tools {
        registry.register(tool).unwrap();
    }
    registry
}

fn engine_that_will_crash(turns: Vec<Turn>, tools: Arc<InMemoryToolRegistry>) -> (Engine, Arc<MemoryRepository>) {
    let repo = Arc::new(MemoryRepository::new());
    let model = Arc::new(ScriptedModel::new(turns));
    (Engine::builder(repo.clone(), model).tools(tools).config(fast_config()).build(), repo)
}

#[tokio::test]
async fn a_crash_during_a_model_call_resumes_from_the_checkpoint_without_repeating_finished_tools() {
    let echo_before = Counting::new("echo", Behaviour::Echo);
    let (first, repo) = engine_that_will_crash(
        vec![Turn::call("echo", json!({"text": "before the crash"})), Turn::Hang],
        registry_of(vec![echo_before.clone()]),
    );
    let started = first.submit(request("go", &["echo"])).await.unwrap();
    let mid = wait_for_state(&first, started.id, S::WaitingForModel).await;
    assert!(
        tokio::time::timeout(Duration::from_secs(5), async {
            while first.get(&local(), started.id).await.unwrap().iteration < 2 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .is_ok()
    );
    assert_eq!(echo_before.count(), 1);
    first.abort();
    drop(mid);

    let echo_after = Counting::new("echo", Behaviour::Echo);
    let (second, model) = restarted(
        &repo,
        vec![Turn::Say("finished after the restart".into())],
        registry_of(vec![echo_after.clone()]),
        true,
    );
    let report = second.recover().await.unwrap();
    assert_eq!(report.requeued, vec![started.id]);
    let done = settled(&second, started.id).await;

    assert_eq!(done.state, S::Completed);
    assert_eq!(done.result.unwrap().text, "finished after the restart");
    assert_eq!(echo_after.count(), 0, "the tool that finished before the crash was not run again");
    let request = model.requests().remove(0);
    assert!(
        request.messages.iter().any(|m| m.tool_call_id.is_some() && m.content.contains("before the crash")),
        "the model sees the result recorded before the crash"
    );
    assert!(kinds(&second, started.id).await.contains(&"execution.recovered"));
}

#[tokio::test]
async fn a_crash_during_a_tool_that_may_be_repeated_runs_it_again() {
    let (first, repo) = engine_that_will_crash(
        vec![Turn::call("slow.read", json!({}))],
        registry_of(vec![Counting::new("slow.read", Behaviour::SlowRead(Duration::from_secs(30)))]),
    );
    let started = first.submit(request("go", &["slow.read"])).await.unwrap();
    wait_for_state(&first, started.id, S::WaitingForTool).await;
    first.abort();

    let again = Counting::new("slow.read", Behaviour::SlowRead(Duration::from_millis(5)));
    let (second, _) = restarted(&repo, vec![Turn::Say("read it".into())], registry_of(vec![again.clone()]), true);
    let report = second.recover().await.unwrap();
    assert_eq!(report.requeued, vec![started.id]);
    assert!(report.needs_decision.is_empty());
    assert_eq!(settled(&second, started.id).await.state, S::Completed);
    assert_eq!(again.count(), 1);
}

#[tokio::test]
async fn a_crash_during_a_tool_that_may_not_be_repeated_asks_a_person_before_running_it_again() {
    let (first, repo) = engine_that_will_crash(
        vec![Turn::call("slow", json!({}))],
        registry_of(vec![Counting::new("slow", Behaviour::Slow(Duration::from_secs(30)))]),
    );
    let started = first.submit(request("go", &["slow"])).await.unwrap();
    wait_for_state(&first, started.id, S::WaitingForTool).await;
    first.abort();

    let again = Counting::new("slow", Behaviour::Slow(Duration::from_millis(5)));
    let (second, _) = restarted(&repo, vec![Turn::Say("done".into())], registry_of(vec![again.clone()]), true);
    let report = second.recover().await.unwrap();
    assert_eq!(report.needs_decision, vec![started.id]);
    assert!(report.requeued.is_empty());

    let waiting = settled(&second, started.id).await;
    assert_eq!(waiting.state, S::WaitingForApproval);
    assert_eq!(again.count(), 0, "not run again without a decision");
    let approval = pending_approval(&second, started.id).await;
    assert!(approval.reason.contains("not known whether it finished"), "{}", approval.reason);
    let events = second.events(&local(), started.id, 0).await.unwrap();
    assert!(events.iter().any(|e| e.payload["outcome_unknown"] == json!(true)));

    answer(&second, started.id, true, None).await;
    assert_eq!(settled(&second, started.id).await.state, S::Completed);
    assert_eq!(again.count(), 1);
}

#[tokio::test]
async fn refusing_to_repeat_an_uncertain_tool_skips_it() {
    let (first, repo) = engine_that_will_crash(
        vec![Turn::call("slow", json!({}))],
        registry_of(vec![Counting::new("slow", Behaviour::Slow(Duration::from_secs(30)))]),
    );
    let started = first.submit(request("go", &["slow"])).await.unwrap();
    wait_for_state(&first, started.id, S::WaitingForTool).await;
    first.abort();

    let again = Counting::new("slow", Behaviour::Slow(Duration::from_millis(5)));
    let (second, model) =
        restarted(&repo, vec![Turn::Say("skipped it".into())], registry_of(vec![again.clone()]), true);
    second.recover().await.unwrap();
    settled(&second, started.id).await;
    answer(&second, started.id, false, Some("do not repeat")).await;
    let done = settled(&second, started.id).await;
    assert_eq!(done.state, S::Completed);
    assert_eq!(again.count(), 0);
    let seen = model.requests().remove(0);
    assert!(seen.messages.iter().any(|m| m.is_error && m.content.contains("a person refused")));
    let record = second.detail(&local(), started.id).await.unwrap().tool_calls.remove(0);
    assert_eq!(record.status, ToolCallStatus::Denied);
}

#[tokio::test]
async fn recovery_leaves_waiting_executions_alone_and_they_can_still_be_answered_after_a_restart() {
    let write_before = Counting::new("files.write", Behaviour::Write);
    let (first, repo) = engine_that_will_crash(
        vec![Turn::call("files.write", json!({"path": "a.txt", "content": "x"}))],
        registry_of(vec![write_before.clone()]),
    );
    let waiting = run(&first, request("save", &["files.write"])).await;
    assert_eq!(waiting.state, S::WaitingForApproval);
    first.shutdown().await;

    let write_after = Counting::new("files.write", Behaviour::Write);
    let (second, _) = restarted(&repo, vec![Turn::Say("saved".into())], registry_of(vec![write_after.clone()]), true);
    let report = second.recover().await.unwrap();
    assert!(report.requeued.is_empty() && report.needs_decision.is_empty());
    assert_eq!(report.untouched, 1);
    assert_eq!(second.get(&local(), waiting.id).await.unwrap().state, S::WaitingForApproval);

    answer(&second, waiting.id, true, None).await;
    assert_eq!(settled(&second, waiting.id).await.state, S::Completed);
    assert_eq!((write_before.count(), write_after.count()), (0, 1), "it ran once, after the restart");
}

#[tokio::test]
async fn queued_and_interrupted_executions_are_both_picked_up_after_a_restart() {
    let repo = Arc::new(MemoryRepository::new());
    let model = Arc::new(ScriptedModel::new([Turn::Hang]));
    let config = EngineConfig { workers: 1, ..fast_config() };
    let first = Engine::builder(repo.clone(), model).config(config).build();
    let busy = first.submit(request("busy", &[])).await.unwrap();
    wait_for_state(&first, busy.id, S::WaitingForModel).await;
    let waiting_in_line = first.submit(request("waiting in line", &[])).await.unwrap();
    assert_eq!(first.get(&local(), waiting_in_line.id).await.unwrap().state, S::Queued);
    first.abort();

    let (second, _) =
        restarted(&repo, vec![Turn::Say("one".into()), Turn::Say("two".into())], registry_of(vec![]), true);
    let report = second.recover().await.unwrap();
    assert_eq!(report.requeued, vec![busy.id]);
    assert_eq!(report.untouched, 1);
    assert_eq!(settled(&second, busy.id).await.state, S::Completed);
    assert_eq!(settled(&second, waiting_in_line.id).await.state, S::Completed);
}

#[tokio::test]
async fn a_graceful_shutdown_leaves_work_as_it_is_so_it_can_be_continued() {
    let (first, repo) = engine_that_will_crash(
        vec![Turn::call("slow.read", json!({}))],
        registry_of(vec![Counting::new("slow.read", Behaviour::SlowRead(Duration::from_secs(30)))]),
    );
    let started = first.submit(request("go", &["slow.read"])).await.unwrap();
    wait_for_state(&first, started.id, S::WaitingForTool).await;
    first.shutdown().await;
    let after = first.get(&local(), started.id).await.unwrap();
    assert_eq!(after.state, S::WaitingForTool, "shutting down is not cancelling");

    let (second, _) = restarted(
        &repo,
        vec![Turn::Say("continued".into())],
        registry_of(vec![Counting::new("slow.read", Behaviour::SlowRead(Duration::from_millis(5)))]),
        true,
    );
    second.recover().await.unwrap();
    assert_eq!(settled(&second, started.id).await.state, S::Completed);
}

#[tokio::test]
async fn a_careful_recovery_waits_for_the_lease_of_a_dead_engine_to_expire() {
    let (first, repo) = engine_that_will_crash(vec![Turn::Hang], registry_of(vec![]));
    let started = first.submit(request("go", &[])).await.unwrap();
    wait_for_state(&first, started.id, S::WaitingForModel).await;
    first.abort();

    // Not the only owner: it cannot tell a dead engine from a slow one until the lease runs out (1 second here).
    let (second, _) = restarted(&repo, vec![Turn::Say("later".into())], registry_of(vec![]), false);
    let early = second.recover().await.unwrap();
    assert_eq!((early.in_use, early.requeued.len()), (1, 0));
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    let later = second.recover().await.unwrap();
    assert_eq!(later.requeued, vec![started.id]);
    assert_eq!(settled(&second, started.id).await.state, S::Completed);
}
