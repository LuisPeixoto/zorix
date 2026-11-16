//! The engine learns how many real tokens its estimates are worth, per model, from what providers report (ADR-0016).

mod common;

use std::sync::Arc;

use async_trait::async_trait;
use common::*;
use tokio_util::sync::CancellationToken;
use zorix_core::{
    ExecutionEngine, ExecutionState as S, FinishReason, InMemoryCalibration, Message, ModelError, ModelGateway,
    ModelRequest, ModelResponse, TokenCalibration, Usage,
};
use zorix_runtime::{Engine, MemoryRepository};

/// Reports that every prompt cost `times` as many tokens as ZORIX estimated, and half of it came from the cache.
struct Honest {
    times: f64,
    provider: Option<&'static str>,
}

#[async_trait]
impl ModelGateway for Honest {
    async fn generate(&self, request: ModelRequest, _cancel: CancellationToken) -> Result<ModelResponse, ModelError> {
        let estimate: u64 =
            request.messages.iter().map(Message::estimated_tokens).sum::<u64>() + request.estimated_tool_tokens();
        let real = (estimate as f64 * self.times) as u64;
        let usage =
            Usage { input_tokens: real / 2, cache_read_tokens: real - real / 2, output_tokens: 5, ..Usage::default() };
        let mut response = ModelResponse::new("the-model", "done".into(), vec![], usage, FinishReason::Stop);
        response.provider = self.provider.map(str::to_string);
        Ok(response)
    }
}

fn long_request(model: &str) -> zorix_core::NewExecution {
    let mut new = request(&"word ".repeat(1_000), &[]);
    new.request.model = model.into();
    new
}

async fn factor_seen(engine: &Engine, id: zorix_core::ExecutionId) -> f64 {
    let events = engine.events(&local(), id, 0).await.unwrap();
    events.iter().find(|e| e.kind.name() == "execution.model.requested").unwrap().payload["token_factor"]
        .as_f64()
        .unwrap()
}

#[tokio::test]
async fn after_a_few_calls_the_engine_uses_the_real_ratio_for_that_model() {
    let calibration = Arc::new(InMemoryCalibration::new());
    let engine =
        Engine::builder(Arc::new(MemoryRepository::new()), Arc::new(Honest { times: 2.0, provider: Some("fake") }))
            .calibration(calibration.clone())
            .config(fast_config())
            .build();
    let mut seen = Vec::new();
    for _ in 0..5 {
        let done = run(&engine, long_request("route:fast")).await;
        assert_eq!(done.state, S::Completed);
        seen.push(factor_seen(&engine, done.id).await);
    }
    assert_eq!(&seen[..3], &[1.0, 1.0, 1.0], "nothing is trusted before three samples");
    assert!((seen[3] - 2.0).abs() < 0.05 && (seen[4] - 2.0).abs() < 0.05, "{seen:?}");
    // The cache is counted: the real prompt size is input plus cache reads.
    assert!((calibration.factor("route:fast") - 2.0).abs() < 0.05);
    // The same data teaches the name of the model that served the calls, too.
    assert!((calibration.factor("fake/the-model") - 2.0).abs() < 0.05);
}

#[tokio::test]
async fn simulated_models_and_short_prompts_teach_nothing() {
    let calibration = Arc::new(InMemoryCalibration::new());
    let engine = Engine::builder(Arc::new(MemoryRepository::new()), Arc::new(Honest { times: 3.0, provider: None }))
        .calibration(calibration.clone())
        .config(fast_config())
        .build();
    for _ in 0..4 {
        let mut new = request(&"word ".repeat(1_000), &[]);
        new.request.model = "demo".into();
        run(&engine, new).await;
        let mut short = request("hi", &[]);
        short.request.model = "fake/the-model".into();
        run(&engine, short).await;
    }
    assert!(calibration.is_empty(), "{:?}", calibration.all());
}

#[tokio::test]
async fn without_a_calibration_estimates_are_used_as_they_are() {
    let engine = Engine::builder(Arc::new(MemoryRepository::new()), Arc::new(Honest { times: 2.0, provider: None }))
        .config(fast_config())
        .build();
    for _ in 0..5 {
        let done = run(&engine, long_request("fake/the-model")).await;
        assert_eq!(factor_seen(&engine, done.id).await, 1.0);
    }
}
