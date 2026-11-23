//! How the loop copes with a model that fails the way real providers do: typed errors, a stream that is cut, answers that are empty,
//! cut short or refused, calls that make no sense, and failures that come and go.

mod common;

use std::sync::Arc;

use common::*;
use futures_util::StreamExt;
use serde_json::json;
use zorix_core::{
    ExecutionEngine, ExecutionState as S, FailureKind, FinishReason, Limits, LiveEvent, ModelError, ModelErrorKind,
};
use zorix_runtime::{Engine, InMemoryToolRegistry, MemoryRepository};
use zorix_sim::{Failure, Faulty, ScriptedModel, Turn};

fn provider(kind: ModelErrorKind, message: &str, retry_after_ms: Option<u64>) -> Turn {
    Turn::Fail(Failure::Kind { kind, message: message.into(), retry_after_ms })
}

#[tokio::test]
async fn a_rate_limit_is_retried_after_the_pause_the_provider_asked_for() {
    let h = harness(vec![provider(ModelErrorKind::RateLimited, "Slow down.", Some(60)), Turn::Say("done".into())]);
    let done = run(&h.engine, request("go", &[])).await;
    assert_eq!(done.state, S::Completed, "{:?}", done.failure);
    let events = h.engine.events(&local(), done.id, 0).await.unwrap();
    let retry = events.iter().find(|e| e.kind.name() == "execution.model.retrying").expect("a retry was recorded");
    assert!(retry.payload["delay_ms"].as_u64().unwrap() >= 60, "{}", retry.payload);
    assert_eq!(retry.payload["error"], "Slow down.");
}

#[tokio::test]
async fn errors_that_cannot_succeed_fail_at_once_with_the_providers_own_sentence() {
    for (kind, message) in [
        (ModelErrorKind::Auth, "The API key was rejected."),
        (ModelErrorKind::ContextTooLong, "The conversation does not fit this model."),
        (ModelErrorKind::Refused, "The provider declined to answer."),
        (ModelErrorKind::NotFound, "There is no such model."),
        (ModelErrorKind::Invalid, "The request is wrong."),
    ] {
        let h = harness(vec![provider(kind, message, None), Turn::Say("never reached".into())]);
        let done = run(&h.engine, request("go", &[])).await;
        assert_eq!(done.state, S::Failed, "{kind:?}");
        let failure = done.failure.unwrap();
        assert_eq!((failure.kind, failure.message.as_str()), (FailureKind::Model, message), "{kind:?}");
        assert_eq!(h.model.requests().len(), 1, "{kind:?} is not retried");
    }
}

#[tokio::test]
async fn a_stream_that_is_cut_is_retried_and_the_reader_is_told_to_forget_what_was_written() {
    let h = harness(vec![
        Turn::Stream {
            chunks: vec!["The answer is ".into(), "ab".into()],
            then: Box::new(Turn::Fail(Failure::Transient("connection reset".into()))),
        },
        Turn::Stream {
            chunks: vec!["The answer ".into(), "is 391.".into()],
            then: Box::new(Turn::Say("The answer is 391.".into())),
        },
    ]);
    let started = h.engine.submit(request("go", &[])).await.unwrap();
    let mut live = h.engine.live(&local(), started.id).await.unwrap();
    let done = settled(&h.engine, started.id).await;
    assert_eq!(done.state, S::Completed, "{:?}", done.failure);
    assert_eq!(done.result.unwrap().text, "The answer is 391.", "the stored answer is the whole one, not the cut one");

    // What a reader of the live text saw, replayed the way an interface does: a reset clears what was shown.
    let mut shown = String::new();
    let mut resets = 0;
    while let Ok(Some(event)) = tokio::time::timeout(std::time::Duration::from_millis(50), live.next()).await {
        match event {
            LiveEvent::Text { delta, .. } => shown.push_str(&delta),
            LiveEvent::Reset { .. } => {
                resets += 1;
                shown.clear();
            }
        }
    }
    assert_eq!(resets, 1, "the cut attempt was forgotten before the next one wrote");
    assert_eq!(shown, "The answer is 391.");
}

#[tokio::test]
async fn an_empty_answer_is_sent_back_once_and_then_fails_instead_of_completing_with_nothing() {
    let h = harness(vec![Turn::Say(String::new()), Turn::Say("   ".into())]);
    let done = run(&h.engine, request("go", &[])).await;
    assert_eq!(done.state, S::Failed);
    let failure = done.failure.unwrap();
    assert_eq!(failure.kind, FailureKind::Validation);
    assert!(failure.message.contains("empty"), "{}", failure.message);
    assert_eq!(h.model.requests().len(), 2, "told once, and it may try again");

    let h = harness(vec![Turn::Say(String::new()), Turn::Say("Here it is.".into())]);
    let done = run(&h.engine, request("go", &[])).await;
    assert_eq!((done.state, done.result.unwrap().text.as_str()), (S::Completed, "Here it is."));
}

#[tokio::test]
async fn an_answer_that_ran_out_of_room_is_not_taken_as_complete() {
    let cut = |text: &str| Turn::Raw { text: text.into(), calls: vec![], finish: FinishReason::Length };
    let h = harness(vec![cut("The first half of the"), cut("A shorter but also cu")]);
    let done = run(&h.engine, request("go", &[])).await;
    assert_eq!(done.state, S::Failed);
    let failure = done.failure.unwrap();
    assert_eq!(failure.kind, FailureKind::Model);
    assert!(failure.message.contains("ran out of room"), "{}", failure.message);

    let h = harness(vec![cut("half"), Turn::Say("A complete, shorter answer.".into())]);
    let done = run(&h.engine, request("go", &[])).await;
    assert_eq!((done.state, done.result.unwrap().text.as_str()), (S::Completed, "A complete, shorter answer."));
    let told = &h.model.requests()[1].messages;
    assert!(told.iter().any(|m| m.content.contains("ran out of room")), "the model is told why it must answer again");
}

#[tokio::test]
async fn a_refusal_fails_the_execution_and_does_not_become_its_result() {
    let h = harness(vec![Turn::Raw {
        text: "I can't help with that.".into(),
        calls: vec![],
        finish: FinishReason::Refusal,
    }]);
    let done = run(&h.engine, request("go", &[])).await;
    assert_eq!(done.state, S::Failed);
    let failure = done.failure.unwrap();
    assert_eq!(failure.kind, FailureKind::Model);
    assert!(
        failure.message.contains("declined") && failure.message.contains("I can't help with that."),
        "{}",
        failure.message
    );
    assert!(done.result.is_none());
}

#[tokio::test]
async fn the_content_decides_when_the_finish_reason_disagrees_with_it() {
    // It says it wants tools but asks for none: the text is the answer.
    let h = harness(vec![Turn::Raw { text: "Just text.".into(), calls: vec![], finish: FinishReason::ToolCalls }]);
    let done = run(&h.engine, request("go", &[])).await;
    assert_eq!((done.state, done.result.unwrap().text.as_str()), (S::Completed, "Just text."));

    // It says it stopped but asks for a tool: the tool runs, once, and the loop goes on.
    let call = zorix_sim::ScriptedCall { tool: "echo".into(), arguments: json!({"text": "hi"}) };
    let h = harness(vec![
        Turn::Raw { text: "Calling.".into(), calls: vec![call], finish: FinishReason::Stop },
        Turn::Say("Done.".into()),
    ]);
    let done = run(&h.engine, request("go", &["echo"])).await;
    assert_eq!((done.state, h.echo.count()), (S::Completed, 1));
}

#[tokio::test]
async fn calls_that_make_no_sense_are_answered_to_the_model_and_never_run() {
    let h = harness(vec![
        Turn::call("no.such.tool", json!({})),
        Turn::call("echo", json!("not an object")),
        Turn::call("echo", json!({"text": 5, "extra": true})),
        Turn::Say("I could not do it.".into()),
    ]);
    let done = run(&h.engine, request("go", &["echo"])).await;
    assert_eq!(done.state, S::Completed, "{:?}", done.failure);
    assert_eq!(h.echo.count(), 0, "none of them ran");
    let seen = h.model.requests();
    let told = |n: usize| seen[n].messages.iter().rev().find(|m| m.tool_call_id.is_some()).unwrap().content.clone();
    assert!(told(1).contains("There is no tool called no.such.tool"), "{}", told(1));
    assert!(told(2).contains("is_error") || told(2).contains("untrusted"), "{}", told(2));
    assert_eq!(seen.len(), 4);
}

#[tokio::test]
async fn failures_that_come_and_go_by_seed_never_run_a_tool_twice_and_the_run_still_finishes() {
    for seed in [1u64, 2, 3, 4, 5, 6, 7, 8] {
        let mut turns = Vec::new();
        for n in 0..4 {
            turns.push(Turn::call("files.write", json!({"path": format!("f{n}"), "content": "x"})));
        }
        turns.push(Turn::Say("all written".into()));
        let model = Arc::new(Faulty::new(ScriptedModel::new(turns)).seeded(seed, 40));
        let (tools, [_, write, ..]) = registry();
        let engine = Engine::builder(Arc::new(MemoryRepository::new()), model.clone())
            .tools(tools as Arc<InMemoryToolRegistry>)
            .config(fast_config())
            .build();
        let new = with_limits(
            request("write four files", &["files.write"]),
            Limits { max_model_attempts: 10, ..Limits::default() },
        );
        let started = engine.submit(new).await.unwrap();
        // Every write asks a person; answer each as it comes.
        let done = loop {
            let current = settled(&engine, started.id).await;
            if current.state != S::WaitingForApproval {
                break current;
            }
            answer(&engine, started.id, true, None).await;
        };
        assert_eq!(done.state, S::Completed, "seed {seed}: {:?}", done.failure);
        assert_eq!(write.count(), 4, "seed {seed}: each write ran exactly once");
        assert!(model.calls() >= 5, "seed {seed}");
    }
    let _ = ModelError::Cancelled;
}
