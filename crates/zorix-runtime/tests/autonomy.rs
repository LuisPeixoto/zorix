//! A worker's level changes what asks and what does not, for runs the workforce starts (ADR-0039).

mod common;

use common::*;
use serde_json::json;
use zorix_core::ExecutionEngine as _;
use zorix_core::{
    Agent, AgentBinding, AgentId, AgentSlug, AgentSpec, AgentVersion, Autonomy, AutonomyLevel, ExecutionState as S,
    Timestamp,
};
use zorix_sim::Turn;

fn write_turn() -> Turn {
    Turn::call("files.write", json!({"path": "report.txt", "content": "hello"}))
}

fn binding(level: AutonomyLevel) -> AgentBinding {
    let slug = AgentSlug::new("forge").unwrap();
    let spec = AgentSpec::new(slug.clone(), "Forge", "sim");
    let agent = Agent {
        id: AgentId::new(),
        tenant: local(),
        slug,
        current_version: 1,
        archived: false,
        created_at: Timestamp(1),
        updated_at: Timestamp(1),
    };
    let version =
        AgentVersion { agent: agent.id, version: 1, hash: spec.hash(), spec, note: None, created_at: Timestamp(1) };
    let mut binding = AgentBinding::of(&agent, &version);
    binding.autonomy = Some(Autonomy::new(level));
    binding
}

fn at(level: AutonomyLevel) -> zorix_core::NewExecution {
    let mut new = request("write the report", &["files.write"]);
    new.agent = Some(binding(level));
    new
}

#[tokio::test]
async fn at_notify_a_call_the_default_would_hold_runs_and_is_recorded_with_its_arguments() {
    let h = harness(vec![write_turn(), Turn::Say("saved".into())]);
    let done = run(&h.engine, at(AutonomyLevel::Notify)).await;
    let done = settled(&h.engine, done.id).await;
    assert_eq!(done.state, S::Completed);
    assert_eq!(h.write.count(), 1, "it ran without a person");
    let all = kinds(&h.engine, done.id).await;
    assert!(
        in_order(&all, &["execution.tool.unattended", "execution.tool.started", "execution.tool.completed"]),
        "{all:?}"
    );
    assert!(!all.contains(&"execution.waiting_approval"));
    let events = h.engine.events(&local(), done.id, 0).await.unwrap();
    let event = events.iter().find(|e| e.kind.name() == "execution.tool.unattended").unwrap();
    assert_eq!(event.payload["tool"], "files.write");
    assert_eq!(event.payload["level"], "notify");
    assert_eq!(event.payload["arguments"], json!({"path": "report.txt", "content": "hello"}));
}

#[tokio::test]
async fn at_suggest_the_call_does_not_run_and_the_proposal_is_recorded() {
    let h = harness(vec![write_turn(), Turn::Say("I proposed it".into())]);
    let done = run(&h.engine, at(AutonomyLevel::Suggest)).await;
    let done = settled(&h.engine, done.id).await;
    assert_eq!(done.state, S::Completed);
    assert_eq!(h.write.count(), 0, "nothing was changed");
    let events = h.engine.events(&local(), done.id, 0).await.unwrap();
    let event = events.iter().find(|e| e.kind.name() == "execution.tool.suggested").expect("a suggestion");
    assert_eq!(event.payload["tool"], "files.write");
    assert_eq!(event.payload["arguments"], json!({"path": "report.txt", "content": "hello"}));
    let told = h.model.requests()[1].messages.iter().rev().find(|m| m.tool_call_id.is_some()).unwrap().clone();
    assert!(told.content.contains("proposed, not run"), "the model hears why: {}", told.content);
}

#[tokio::test]
async fn at_ask_the_call_waits_for_a_person_as_before() {
    let h = harness(vec![write_turn(), Turn::Say("saved".into())]);
    let waiting = run(&h.engine, at(AutonomyLevel::Ask)).await;
    assert_eq!(waiting.state, S::WaitingForApproval);
    assert_eq!(h.write.count(), 0);
}

#[tokio::test]
async fn a_run_nobody_gave_a_level_keeps_asking() {
    let h = harness(vec![write_turn(), Turn::Say("saved".into())]);
    let waiting = run(&h.engine, request("write the report", &["files.write"])).await;
    assert_eq!(waiting.state, S::WaitingForApproval);
}
