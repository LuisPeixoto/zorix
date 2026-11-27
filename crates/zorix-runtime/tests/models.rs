//! How the runtime uses what it knows about a model: its window, phases, native turns, routes, errors and cost.

mod common;

use std::sync::Arc;

use async_trait::async_trait;
use common::*;
use parking_lot::Mutex;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use zorix_core::{
    Effort, ExecutionEngine, ExecutionState as S, FinishReason, GenerationParams, LimitKind, Limits, ModelError,
    ModelErrorKind, ModelGateway, ModelProfile, ModelRequest, ModelResponse, RouteAttempt, RouteTrace, StepPhase,
    Usage,
};
use zorix_runtime::{Engine, MemoryRepository};
use zorix_sim::{Profiled, ScriptedModel, Turn};

fn engine_with(model: Arc<dyn ModelGateway>) -> Engine {
    let (tools, _) = registry();
    Engine::builder(Arc::new(MemoryRepository::new()), model).tools(tools).config(fast_config()).build()
}

#[tokio::test]
async fn a_small_context_window_makes_the_request_say_its_history_was_edited() {
    let big = "x".repeat(2_000);
    let turns: Vec<Turn> =
        (0..5).map(|_| Turn::call("echo", json!({"text": big}))).chain([Turn::Say("done".into())]).collect();
    let model = Arc::new(Profiled::new(
        ScriptedModel::new(turns),
        ModelProfile { context_window: 3_200, max_output: 200, tools: true, vision: false, priced: true },
    ));
    let engine = engine_with(model.clone());
    let done = run(&engine, request("echo a lot", &["echo"])).await;
    assert_eq!(done.state, S::Completed);
    let requests = model.inner().requests();
    assert!(!requests[0].history_edited && !requests[1].history_edited, "early requests fit");
    assert!(requests.last().unwrap().history_edited, "the last one had to drop old turns");
    assert!(requests.last().unwrap().messages.iter().any(|m| m.content.contains("left out to fit")));
}

#[tokio::test]
async fn a_known_large_window_keeps_the_whole_history() {
    let model = Arc::new(Profiled::new(
        ScriptedModel::new([
            Turn::call("echo", json!({"text": "a"})),
            Turn::call("echo", json!({"text": "b"})),
            Turn::Say("ok".into()),
        ]),
        ModelProfile { context_window: 1_000_000, max_output: 64_000, tools: true, vision: true, priced: true },
    ));
    let engine = engine_with(model.clone());
    run(&engine, request("go", &["echo"])).await;
    assert!(model.inner().requests().iter().all(|r| !r.history_edited));
}

#[tokio::test]
async fn the_phase_tells_a_router_where_it_is_in_the_run() {
    let h = harness(vec![
        Turn::call("echo", json!({"text": "x"})),
        Turn::Say("not json".into()),
        Turn::Say("{\"ok\": true}".into()),
    ]);
    let mut new = request("answer in JSON", &["echo"]);
    new.request.expected_output = Some(json!({"type": "object", "required": ["ok"]}));
    let done = run(&h.engine, new).await;
    assert_eq!(done.state, S::Completed);
    let phases: Vec<StepPhase> = h.model.requests().iter().map(|r| r.phase).collect();
    assert_eq!(phases, vec![StepPhase::Start, StepPhase::Continue, StepPhase::Retry]);
}

#[tokio::test]
async fn generation_params_of_the_task_reach_the_model() {
    let h = harness(vec![Turn::Say("ok".into())]);
    let mut new = request("go", &[]);
    new.request.params = GenerationParams { max_output_tokens: Some(777), effort: Some(Effort::High) };
    run(&h.engine, new).await;
    let params = h.model.requests()[0].params;
    assert_eq!((params.max_output_tokens, params.effort), (Some(777), Some(Effort::High)));
}

#[tokio::test]
async fn the_providers_raw_turn_is_kept_and_sent_back_unchanged() {
    let model = Arc::new(
        ScriptedModel::new([Turn::call("echo", json!({"text": "x"})), Turn::Say("done".into())]).with_native("fake"),
    );
    let engine = engine_with(model.clone());
    run(&engine, request("go", &["echo"])).await;
    let second = &model.requests()[1];
    let assistant = second.messages.iter().find(|m| !m.tool_calls.is_empty()).expect("the earlier assistant turn");
    let native = assistant.native.as_ref().expect("its native turn comes back with it");
    assert_eq!((native.provider.as_str(), native.content["text"].as_str()), ("fake", Some("")));
}

/// A gateway that answers with a route trace, as a router would after passing over a candidate.
struct Routed;

#[async_trait]
impl ModelGateway for Routed {
    async fn generate(&self, _request: ModelRequest, _cancel: CancellationToken) -> Result<ModelResponse, ModelError> {
        let mut response = ModelResponse::new(
            "local-model",
            "answered".into(),
            vec![],
            Usage { input_tokens: 10, output_tokens: 5, cost_micros: 0, ..Usage::default() },
            FinishReason::Stop,
        );
        response.provider = Some("ollama".into());
        response.route = Some(RouteTrace {
            attempts: vec![
                RouteAttempt {
                    provider: "anthropic".into(),
                    model: "claude-opus-5-5".into(),
                    outcome: "rate_limited".into(),
                },
                RouteAttempt { provider: "ollama".into(), model: "local-model".into(), outcome: "ok".into() },
            ],
        });
        Ok(response)
    }
}

#[tokio::test]
async fn a_fallback_is_recorded_as_an_event_and_shows_in_the_summary() {
    let engine = engine_with(Arc::new(Routed));
    let done = run(&engine, request("go", &[])).await;
    assert_eq!(done.state, S::Completed);
    let events = engine.events(&local(), done.id, 0).await.unwrap();
    let fallback = events.iter().find(|e| e.kind.name() == "execution.model.fallback").expect("a fallback event");
    assert_eq!(fallback.payload["from"], "anthropic/claude-opus-5-5");
    assert_eq!(fallback.payload["to"], "ollama/local-model");
    assert_eq!(fallback.payload["reason"], "rate_limited");
    let all: Vec<&str> = events.iter().map(|e| e.kind.name()).collect();
    assert!(
        in_order(&all, &["execution.model.fallback", "execution.model.responded", "execution.completed"]),
        "{all:?}"
    );

    let summary = engine.summary(&local(), done.id).await.unwrap();
    assert_eq!((summary.fallbacks, summary.models.as_slice()), (1, ["ollama/local-model".to_string()].as_slice()));
    assert_eq!(summary.steps[0].provider.as_deref(), Some("ollama"));
}

/// A gateway that fails in a chosen way a number of times, then answers.
struct Failing {
    kind: ModelErrorKind,
    times: Mutex<u32>,
    retry_after_ms: Option<u64>,
}

#[async_trait]
impl ModelGateway for Failing {
    async fn generate(&self, _request: ModelRequest, _cancel: CancellationToken) -> Result<ModelResponse, ModelError> {
        let mut left = self.times.lock();
        if *left > 0 {
            *left -= 1;
            return Err(ModelError::Provider {
                kind: self.kind,
                message: format!("it failed: {}", self.kind.as_str()),
                retry_after_ms: self.retry_after_ms,
            });
        }
        Ok(ModelResponse::new("m", "finally".into(), vec![], Usage::default(), FinishReason::Stop))
    }
}

#[tokio::test]
async fn a_temporary_provider_error_is_retried_and_a_final_one_is_not() {
    let rate_limited = Failing { kind: ModelErrorKind::RateLimited, times: Mutex::new(2), retry_after_ms: Some(1) };
    let engine = engine_with(Arc::new(rate_limited));
    let done = run(&engine, request("go", &[])).await;
    assert_eq!(done.state, S::Completed);
    let retries = engine
        .events(&local(), done.id, 0)
        .await
        .unwrap()
        .iter()
        .filter(|e| e.kind.name() == "execution.model.retrying")
        .count();
    assert_eq!(retries, 2);

    for kind in
        [ModelErrorKind::Auth, ModelErrorKind::NotFound, ModelErrorKind::ContextTooLong, ModelErrorKind::Refused]
    {
        let gateway = Arc::new(Failing { kind, times: Mutex::new(9), retry_after_ms: None });
        let engine = engine_with(gateway.clone());
        let done = run(&engine, request("go", &[])).await;
        assert_eq!(done.state, S::Failed, "{kind:?}");
        assert_eq!(done.failure.unwrap().message, format!("it failed: {}", kind.as_str()));
        assert_eq!(*gateway.times.lock(), 8, "{kind:?} was tried once, not retried");
    }
}

#[tokio::test]
async fn the_cost_limit_stops_a_run_that_costs_too_much() {
    let h = harness(vec![Turn::call("echo", json!({"text": "x"})), Turn::Say("never".into())]);
    let new = with_limits(request("go", &["echo"]), Limits { max_cost_micros: Some(1), ..Limits::default() });
    let done = run(&h.engine, new).await;
    assert_eq!(done.failure.unwrap().limit, Some(LimitKind::Cost));
    assert!(done.usage.cost_micros > 1);
}

/// A cost limit counts what each call costs, and a model with no known price costs nothing every time: it would never trip. The run is
/// refused before anything is sent, with the way out; a limit on a priced model, and a model with no limit, are not affected
/// (finding F11 of the Phase 11 audit).
#[tokio::test]
async fn a_cost_limit_on_a_model_with_no_known_price_is_refused_before_anything_is_sent() {
    let profile =
        |priced| ModelProfile { context_window: 100_000, max_output: 4_000, tools: true, vision: false, priced };
    let ask = |limits: Limits| with_limits(request("go", &[]), limits);
    let with_cost_limit = Limits { max_cost_micros: Some(5_000_000), ..Limits::default() };

    let unpriced = Arc::new(Profiled::new(ScriptedModel::new([Turn::Say("never asked".into())]), profile(false)));
    let done = run(&engine_with(unpriced.clone()), ask(with_cost_limit)).await;
    assert_eq!(done.state, S::Failed);
    let failure = done.failure.unwrap();
    assert_eq!(failure.limit, Some(LimitKind::Cost));
    assert!(
        failure.message.contains("price of sim is not known") && failure.message.contains("providers.toml"),
        "{}",
        failure.message
    );
    assert!(unpriced.inner().requests().is_empty(), "nothing was sent to the model");

    let priced = Arc::new(Profiled::new(ScriptedModel::new([Turn::Say("ok".into())]), profile(true)));
    assert_eq!(run(&engine_with(priced), ask(with_cost_limit)).await.state, S::Completed);

    let no_limit = Arc::new(Profiled::new(ScriptedModel::new([Turn::Say("ok".into())]), profile(false)));
    assert_eq!(
        run(&engine_with(no_limit), ask(Limits::default())).await.state,
        S::Completed,
        "without a cost limit nothing needs a price"
    );
}
