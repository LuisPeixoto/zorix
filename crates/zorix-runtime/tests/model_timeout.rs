//! The time limit of a model call is the time without progress, not the time the whole answer takes: an answer that keeps arriving is
//! never cut, and one that stops arriving is (finding F7 of the Phase 11 audit).

mod common;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use common::*;
use tokio_util::sync::CancellationToken;
use zorix_core::{
    ExecutionState as S, FinishReason, Limits, ModelError, ModelGateway, ModelRequest, ModelResponse, TextSink, Usage,
};
use zorix_runtime::{Engine, MemoryRepository};

/// A model that writes its answer piece by piece, `gap_ms` apart, and then either finishes or goes silent.
struct Streaming {
    pieces: usize,
    gap_ms: u64,
    then_silence: bool,
}

#[async_trait]
impl ModelGateway for Streaming {
    async fn generate(&self, request: ModelRequest, cancel: CancellationToken) -> Result<ModelResponse, ModelError> {
        self.generate_live(request, cancel, Arc::new(zorix_core::NoSink)).await
    }

    async fn generate_live(
        &self,
        _request: ModelRequest,
        cancel: CancellationToken,
        sink: Arc<dyn TextSink>,
    ) -> Result<ModelResponse, ModelError> {
        for _ in 0..self.pieces {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(self.gap_ms)) => {}
                _ = cancel.cancelled() => return Err(ModelError::Cancelled),
            }
            sink.text("word ");
        }
        if self.then_silence {
            cancel.cancelled().await;
            return Err(ModelError::Cancelled);
        }
        Ok(ModelResponse::new("sim", "word ".repeat(self.pieces), Vec::new(), Usage::default(), FinishReason::Stop))
    }
}

fn engine_over(model: Streaming) -> Engine {
    Engine::builder(Arc::new(MemoryRepository::new()), Arc::new(model)).config(fast_config()).build()
}

fn limit(ms: u64) -> Limits {
    Limits { model_timeout_ms: ms, max_model_attempts: 1, ..Limits::default() }
}

#[tokio::test]
async fn an_answer_that_keeps_arriving_is_not_cut_although_it_takes_longer_than_the_limit() {
    // Eight pieces 60 ms apart take about 480 ms; the limit is 200 ms without a piece.
    let engine = engine_over(Streaming { pieces: 8, gap_ms: 60, then_silence: false });
    let done = run(&engine, with_limits(request("write a long answer", &[]), limit(200))).await;
    assert_eq!(done.state, S::Completed, "{:?}", done.failure);
    assert_eq!(done.usage.model_calls, 1, "it was not cut and asked again");
}

#[tokio::test]
async fn an_answer_that_stops_arriving_is_cut_after_the_limit_with_no_progress() {
    // Two pieces, then silence: 40 ms of work and then nothing for longer than the 200 ms limit.
    let engine = engine_over(Streaming { pieces: 2, gap_ms: 20, then_silence: true });
    let done = run(&engine, with_limits(request("write", &[]), limit(200))).await;
    assert_eq!(done.state, S::Failed);
    let message = done.failure.unwrap().message;
    assert!(message.contains("did not answer within 200 ms"), "{message}");
}
