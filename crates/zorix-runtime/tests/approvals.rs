//! Human in the loop: suspension, resuming after a decision, and refusing forged or repeated answers.

mod common;

use common::*;
use serde_json::json;
use zorix_core::{
    ApprovalDecision, ApprovalId, ApprovalStatus, Error, ExecutionEngine, ExecutionState as S, TenantId, ToolCallStatus,
};
use zorix_sim::Turn;

fn write_turn() -> Turn {
    Turn::call("files.write", json!({"path": "report.txt", "content": "hello"}))
}

#[tokio::test]
async fn a_destructive_call_suspends_the_execution_until_a_person_answers() {
    let h = harness(vec![write_turn(), Turn::Say("saved".into())]);
    let waiting = run(&h.engine, request("write the report", &["files.write"])).await;
    assert_eq!(waiting.state, S::WaitingForApproval);
    assert_eq!(h.write.count(), 0, "nothing happened yet");

    let approval = pending_approval(&h.engine, waiting.id).await;
    assert_eq!(approval.tool, "files.write");
    assert_eq!(approval.arguments, json!({"path": "report.txt", "content": "hello"}));
    assert_eq!(approval.call_hash, zorix_core::call_hash("files.write", &approval.arguments));
    assert!(kinds(&h.engine, waiting.id).await.contains(&"execution.waiting_approval"));
    let record = &h.engine.detail(&local(), waiting.id).await.unwrap().tool_calls[0];
    assert_eq!(record.status, ToolCallStatus::AwaitingApproval);

    answer(&h.engine, waiting.id, true, None).await;
    let done = settled(&h.engine, waiting.id).await;
    assert_eq!(done.state, S::Completed);
    assert_eq!(h.write.count(), 1, "it ran once, after the approval");
    let all = kinds(&h.engine, waiting.id).await;
    assert!(
        in_order(
            &all,
            &[
                "execution.waiting_approval",
                "execution.approval.granted",
                "execution.tool.started",
                "execution.tool.completed",
                "execution.completed"
            ]
        ),
        "{all:?}"
    );
    let approval = h.engine.detail(&local(), waiting.id).await.unwrap().approvals.remove(0);
    assert_eq!((approval.status, approval.decided_by.as_deref()), (ApprovalStatus::Approved, Some("tester")));
}

#[tokio::test]
async fn a_refused_call_never_runs_and_the_model_hears_why() {
    let h = harness(vec![write_turn(), Turn::Say("understood, skipping".into())]);
    let waiting = run(&h.engine, request("write the report", &["files.write"])).await;
    answer(&h.engine, waiting.id, false, Some("not today")).await;
    let done = settled(&h.engine, waiting.id).await;
    assert_eq!(done.state, S::Completed);
    assert_eq!(h.write.count(), 0);
    let message = h.model.requests()[1].messages.iter().rev().find(|m| m.tool_call_id.is_some()).unwrap().clone();
    assert!(message.is_error && message.content.contains("a person refused") && message.content.contains("not today"));
    assert!(in_order(&kinds(&h.engine, done.id).await, &["execution.approval.denied", "execution.tool.denied"]));
}

#[tokio::test]
async fn an_approval_applies_to_the_call_it_names_and_cannot_be_forged_or_repeated() {
    let h = harness(vec![write_turn(), Turn::Say("saved".into())]);
    let waiting = run(&h.engine, request("write the report", &["files.write"])).await;
    let approval = pending_approval(&h.engine, waiting.id).await;
    let yes = |hash: Option<&str>| ApprovalDecision {
        approve: true,
        decided_by: "tester".into(),
        note: None,
        call_hash: hash.map(str::to_string),
        automatic: false,
    };

    // The answer was given for a different call than the one waiting.
    let wrong_call = h.engine.decide(&local(), waiting.id, approval.id, yes(Some("deadbeef"))).await;
    assert!(matches!(wrong_call, Err(Error::InvalidInput(_))), "{wrong_call:?}");
    // An answer that does not name the call is refused, even for the right approval, and the call stays pending.
    let nameless = h.engine.decide(&local(), waiting.id, approval.id, yes(None)).await;
    assert!(matches!(&nameless, Err(Error::InvalidInput(m)) if m.contains("must name the call")), "{nameless:?}");
    assert_eq!(pending_approval(&h.engine, waiting.id).await.id, approval.id, "still waiting");
    // An id the runtime never issued (for example one the model invented).
    let invented = h.engine.decide(&local(), waiting.id, ApprovalId::new(), yes(None)).await;
    assert!(matches!(invented, Err(Error::Missing(_))), "{invented:?}");
    // Another tenant.
    let stranger = h.engine.decide(&TenantId::new("globex").unwrap(), waiting.id, approval.id, yes(None)).await;
    assert!(matches!(stranger, Err(Error::NotFound(_))));
    // Nobody named themselves.
    let anonymous = ApprovalDecision { decided_by: "  ".into(), ..yes(None) };
    assert!(matches!(h.engine.decide(&local(), waiting.id, approval.id, anonymous).await, Err(Error::InvalidInput(_))));
    assert_eq!(h.write.count(), 0, "none of that ran the tool");

    h.engine.decide(&local(), waiting.id, approval.id, yes(Some(&approval.call_hash))).await.unwrap();
    let again = h.engine.decide(&local(), waiting.id, approval.id, yes(None)).await;
    assert!(matches!(again, Err(Error::InvalidInput(_))), "answering twice is refused: {again:?}");
    settled(&h.engine, waiting.id).await;
    assert_eq!(h.write.count(), 1, "exactly once");
}

#[tokio::test]
async fn deciding_on_an_execution_that_is_not_waiting_is_refused() {
    let h = harness(vec![]);
    let created = h.engine.create(request("x", &[])).await.unwrap();
    let decision =
        ApprovalDecision { approve: true, decided_by: "tester".into(), note: None, call_hash: None, automatic: false };
    let result = h.engine.decide(&local(), created.id, ApprovalId::new(), decision).await;
    assert!(matches!(result, Err(Error::InvalidInput(_))), "{result:?}");
}

#[tokio::test]
async fn cancelling_a_waiting_execution_closes_its_open_approval() {
    let h = harness(vec![write_turn()]);
    let waiting = run(&h.engine, request("write", &["files.write"])).await;
    let cancelled = h.engine.cancel(&local(), waiting.id, Some("never mind".into())).await.unwrap();
    assert_eq!(cancelled.state, S::Cancelled);
    let approval = h.engine.detail(&local(), waiting.id).await.unwrap().approvals.remove(0);
    assert_eq!((approval.status, approval.decided_by.as_deref()), (ApprovalStatus::Denied, Some("system")));
    assert_eq!(h.write.count(), 0);
}

#[tokio::test]
async fn pausing_a_waiting_execution_is_not_allowed_because_it_is_already_suspended() {
    let h = harness(vec![write_turn()]);
    let waiting = run(&h.engine, request("write", &["files.write"])).await;
    assert!(matches!(h.engine.pause(&local(), waiting.id).await, Err(Error::InvalidTransition(_))));
}

/// A tool whose approval must come from a person who looked at it.
struct PersonOnly(zorix_core::ToolSpec, std::sync::atomic::AtomicU32);

#[async_trait::async_trait]
impl zorix_core::Tool for PersonOnly {
    fn spec(&self) -> &zorix_core::ToolSpec {
        &self.0
    }

    async fn call(
        &self,
        _ctx: &zorix_core::ToolContext,
        _args: serde_json::Value,
    ) -> Result<serde_json::Value, zorix_core::ToolError> {
        self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(json!({"saved": true}))
    }
}

#[tokio::test]
async fn a_tool_that_needs_a_person_is_never_approved_by_an_automatic_mode() {
    use std::sync::Arc;
    use zorix_core::ToolRegistry;
    let mut spec =
        zorix_core::ToolSpec::new("memory.remember", "Remember", "Remember something", json!({"type": "object"}));
    spec.requires_approval = true;
    spec.approval_by_person_only = true;
    let tool = Arc::new(PersonOnly(spec, Default::default()));
    let registry = Arc::new(zorix_runtime::InMemoryToolRegistry::new());
    registry.register(tool.clone()).unwrap();
    let model = Arc::new(zorix_sim::ScriptedModel::new([
        Turn::call("memory.remember", json!({})),
        Turn::call("memory.remember", json!({})),
        Turn::Say("remembered".into()),
        Turn::Say("understood".into()),
    ]));
    let engine = zorix_runtime::Engine::builder(Arc::new(zorix_runtime::MemoryRepository::new()), model.clone())
        .tools(registry)
        .config(fast_config())
        .build();
    let waiting = run(&engine, request("remember this", &["memory.remember"])).await;
    assert_eq!(waiting.state, S::WaitingForApproval);
    // The event tells hosts that only a person may answer.
    let events = engine.events(&local(), waiting.id, 0).await.unwrap();
    let event = events.iter().find(|e| e.kind.name() == "execution.waiting_approval").unwrap();
    assert_eq!(event.payload["person_only"], json!(true));

    let approval = pending_approval(&engine, waiting.id).await;
    let decision = |approve: bool, automatic: bool| ApprovalDecision {
        approve,
        decided_by: "cli:auto".into(),
        note: None,
        call_hash: Some(approval.call_hash.clone()),
        automatic,
    };
    let refused = engine.decide(&local(), waiting.id, approval.id, decision(true, true)).await;
    assert!(matches!(&refused, Err(Error::InvalidInput(m)) if m.contains("approved by a person")), "{refused:?}");
    assert_eq!(tool.1.load(std::sync::atomic::Ordering::SeqCst), 0, "the tool did not run");
    assert_eq!(settled(&engine, waiting.id).await.state, S::WaitingForApproval, "it is still waiting");
    // An automatic refusal is fine, and a person's approval works.
    let other = run(&engine, request("remember this too", &["memory.remember"])).await;
    assert_eq!(other.state, S::WaitingForApproval, "{:?}", other.failure);
    let approval2 = pending_approval(&engine, other.id).await;
    let no = ApprovalDecision {
        approve: false,
        decided_by: "cli:auto".into(),
        note: None,
        call_hash: Some(approval2.call_hash.clone()),
        automatic: true,
    };
    engine.decide(&local(), other.id, approval2.id, no).await.unwrap();
    engine.decide(&local(), waiting.id, approval.id, decision(true, false)).await.unwrap();
    assert_eq!(settled(&engine, waiting.id).await.state, S::Completed);
    assert_eq!(tool.1.load(std::sync::atomic::Ordering::SeqCst), 1, "it ran once, after a person approved");
}
