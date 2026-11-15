//! What can be seen from outside: the summary of a run, the trace spans, and what never shows up in logs.

mod common;

use common::*;
use serde_json::json;
use zorix_core::{ExecutionEngine, ExecutionState as S, ToolCallStatus};
use zorix_sim::Turn;

#[tokio::test]
async fn the_summary_says_which_model_and_tools_ran_how_long_and_what_it_cost() {
    let h = harness(vec![Turn::call("echo", json!({"text": "hi"})), Turn::Say("All done.".into())]);
    let done = run(&h.engine, request("echo hi", &["echo"])).await;
    let summary = h.engine.summary(&local(), done.id).await.unwrap();

    assert_eq!((summary.state, summary.model.as_str()), (S::Completed, "sim"));
    assert_eq!((summary.iterations, summary.model_calls, summary.model_retries), (2, 2, 0));
    assert_eq!(summary.tool_calls.len(), 1);
    assert_eq!(summary.tool_calls[0].tool, "echo");
    assert_eq!((summary.tool_calls[0].status, summary.tool_calls[0].attempts), (ToolCallStatus::Succeeded, 1));
    let kinds: Vec<&str> = summary.steps.iter().map(|s| s.kind.as_str()).collect();
    assert_eq!(kinds, vec!["model", "tool", "model"]);
    assert!(summary.usage.total_tokens() > 0 && summary.usage.cost_micros > 0);
    assert_eq!(summary.result_preview.as_deref(), Some("All done."));
    assert!(summary.total_ms.is_some() && summary.failure.is_none());
    assert_eq!(summary.events, h.engine.events(&local(), done.id, 0).await.unwrap().len());
    assert!(serde_json::to_string(&summary).is_ok());
}

#[tokio::test]
async fn the_summary_of_a_failed_run_carries_the_reason_and_the_retries() {
    let h = harness(vec![
        Turn::Fail(zorix_sim::Failure::Transient("busy".into())),
        Turn::Fail(zorix_sim::Failure::Permanent("bad key".into())),
    ]);
    let done = run(&h.engine, request("go", &[])).await;
    let summary = h.engine.summary(&local(), done.id).await.unwrap();
    assert_eq!((summary.state, summary.model_retries), (S::Failed, 1));
    assert_eq!(summary.failure.unwrap().message, "bad key");
    assert!(summary.result_preview.is_none());
}
