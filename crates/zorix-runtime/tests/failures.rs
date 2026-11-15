//! Failures, retries, limits and the validation of final answers.

mod common;

use common::*;
use serde_json::json;
use zorix_core::{ExecutionState as S, FailureKind, LimitKind, Limits};
use zorix_sim::{Failure as Script, Turn};

#[tokio::test]
async fn a_permanent_model_error_fails_the_execution_with_the_reason() {
    let h = harness(vec![Turn::Fail(Script::Permanent("The key was rejected.".into()))]);
    let done = run(&h.engine, request("go", &[])).await;
    assert_eq!(done.state, S::Failed);
    let failure = done.failure.unwrap();
    assert_eq!((failure.kind, failure.message.as_str()), (FailureKind::Model, "The key was rejected."));
    assert_eq!(h.model.requests().len(), 1, "a permanent error is not retried");
    assert_eq!(kinds(&h.engine, done.id).await.last(), Some(&"execution.failed"));
    assert!(done.finished_at.is_some() && done.result.is_none());
}

#[tokio::test]
async fn temporary_model_errors_are_retried_and_the_execution_still_completes() {
    let h = harness(vec![
        Turn::Fail(Script::Transient("busy".into())),
        Turn::Fail(Script::Transient("busy".into())),
        Turn::Say("finally".into()),
    ]);
    let done = run(&h.engine, request("go", &[])).await;
    assert_eq!(done.state, S::Completed);
    assert_eq!(h.model.requests().len(), 3);
    let retries = kinds(&h.engine, done.id).await.iter().filter(|k| **k == "execution.model.retrying").count();
    assert_eq!(retries, 2);
    assert_eq!(done.usage.model_calls, 1, "only the call that answered is counted");
}

#[tokio::test]
async fn giving_up_after_the_allowed_attempts_is_a_failure() {
    let h = harness(vec![Turn::Fail(Script::Transient("busy".into())); 3]);
    let done = run(&h.engine, request("go", &[])).await;
    assert_eq!(done.state, S::Failed);
    let failure = done.failure.unwrap();
    assert_eq!(failure.kind, FailureKind::Model);
    assert!(failure.message.contains("3 times") && failure.message.contains("busy"), "{}", failure.message);
}

#[tokio::test]
async fn a_model_call_that_takes_too_long_times_out_and_counts_as_a_temporary_error() {
    let h = harness(vec![Turn::Hang]);
    let new =
        with_limits(request("go", &[]), Limits { model_timeout_ms: 40, max_model_attempts: 1, ..Limits::default() });
    let done = run(&h.engine, new).await;
    assert_eq!(done.state, S::Failed);
    assert!(done.failure.unwrap().message.contains("did not answer within 40 ms"));
}

#[tokio::test]
async fn the_iteration_limit_stops_a_loop_that_never_ends() {
    let turns = (0..10).map(|_| Turn::call("echo", json!({"text": "again"}))).collect();
    let h = harness(turns);
    let new = with_limits(request("loop", &["echo"]), Limits { max_iterations: 3, ..Limits::default() });
    let done = run(&h.engine, new).await;
    assert_eq!(done.state, S::Failed);
    let failure = done.failure.unwrap();
    assert_eq!((failure.kind, failure.limit), (FailureKind::LimitExceeded, Some(LimitKind::Iterations)));
    assert_eq!(h.model.requests().len(), 3);
    assert_eq!(h.echo.count(), 3);
}

#[tokio::test]
async fn the_tool_call_limit_stops_the_calls_before_they_run() {
    let five = Turn::Call {
        text: String::new(),
        calls: (0..5)
            .map(|_| zorix_sim::ScriptedCall { tool: "echo".into(), arguments: json!({"text": "x"}) })
            .collect(),
    };
    let h = harness(vec![five]);
    let new = with_limits(request("many", &["echo"]), Limits { max_tool_calls: 2, ..Limits::default() });
    let done = run(&h.engine, new).await;
    assert_eq!(done.failure.unwrap().limit, Some(LimitKind::ToolCalls));
    assert_eq!(h.echo.count(), 2, "the third call was refused before it ran");
}

#[tokio::test]
async fn the_token_limit_stops_a_costly_run() {
    let h = harness(vec![Turn::call("echo", json!({"text": "x"})), Turn::Say("never reached".into())]);
    let new = with_limits(request("go", &["echo"]), Limits { max_total_tokens: Some(1), ..Limits::default() });
    let done = run(&h.engine, new).await;
    assert_eq!(done.failure.unwrap().limit, Some(LimitKind::Tokens));
    assert_eq!(h.model.requests().len(), 1);
}

#[tokio::test]
async fn the_time_limit_counts_working_time() {
    let h = harness(vec![
        Turn::Delay { ms: 60, then: Box::new(Turn::call("echo", json!({"text": "x"}))) },
        Turn::Say("late".into()),
    ]);
    let new = with_limits(request("go", &["echo"]), Limits { max_active_ms: 10, ..Limits::default() });
    let done = run(&h.engine, new).await;
    assert_eq!(done.failure.unwrap().limit, Some(LimitKind::ActiveTime));
}

#[tokio::test]
async fn waiting_for_a_person_does_not_use_up_the_time_limit() {
    let h = harness(vec![Turn::call("files.write", json!({"path": "a", "content": "b"})), Turn::Say("done".into())]);
    let new = with_limits(request("go", &["files.write"]), Limits { max_active_ms: 400, ..Limits::default() });
    let waiting = run(&h.engine, new).await;
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    answer(&h.engine, waiting.id, true, None).await;
    let done = settled(&h.engine, waiting.id).await;
    assert_eq!(done.state, S::Completed, "{:?}", done.failure);
    assert!(done.active_ms < 400);
}

#[tokio::test]
async fn a_rejected_answer_is_sent_back_once_with_the_reason_and_can_then_succeed() {
    let h = harness(vec![Turn::Say("it is 391".into()), Turn::Say("{\"total\": 391}".into())]);
    let mut new = request("give me the total as JSON", &[]);
    new.request.expected_output =
        Some(json!({"type": "object", "required": ["total"], "properties": {"total": {"type": "number"}}}));
    let done = run(&h.engine, new).await;
    assert_eq!(done.state, S::Completed);
    assert_eq!(done.result.unwrap().structured, Some(json!({"total": 391})));
    assert_eq!(done.validation_retries, 1);
    let second = &h.model.requests()[1];
    let feedback =
        second.messages.iter().find(|m| m.content.starts_with("Your answer was rejected")).expect("feedback");
    assert!(feedback.content.contains("JSON"));
    assert!(kinds(&h.engine, done.id).await.contains(&"execution.result.rejected"));
}

#[tokio::test]
async fn an_answer_that_keeps_failing_validation_fails_the_execution() {
    let h = harness(vec![Turn::Say("nope".into()), Turn::Say("still nope".into())]);
    let mut new = request("give JSON", &[]);
    new.request.expected_output = Some(json!({"type": "object"}));
    let done = run(&h.engine, new).await;
    assert_eq!(done.state, S::Failed);
    assert_eq!(done.failure.unwrap().kind, FailureKind::Validation);
}

#[tokio::test]
async fn an_empty_answer_is_rejected() {
    let h = harness(vec![Turn::Say("   ".into()), Turn::Say("Now I have something to say.".into())]);
    let done = run(&h.engine, request("go", &[])).await;
    assert_eq!(done.state, S::Completed);
    assert_eq!(done.result.unwrap().text, "Now I have something to say.");
}

#[tokio::test]
async fn a_script_that_runs_out_fails_cleanly_instead_of_hanging() {
    let h = harness(vec![]);
    let done = run(&h.engine, request("go", &[])).await;
    assert_eq!(done.state, S::Failed);
    assert!(done.failure.unwrap().message.contains("no more turns"));
}
