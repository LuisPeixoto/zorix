//! Failure injection: wraps any model and makes it slow or unreliable, deterministically.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use zorix_core::{ModelError, ModelGateway, ModelRequest, ModelResponse};

/// Makes the first calls fail with a temporary error and adds latency to every call.
pub struct Faulty<M> {
    inner: M,
    fail_first: AtomicU32,
    retry_after_ms: Option<u64>,
    latency: Duration,
    calls: AtomicU32,
}

impl<M: ModelGateway> Faulty<M> {
    /// Wrap a model. It behaves like the inner one until configured.
    pub fn new(inner: M) -> Self {
        Self {
            inner,
            fail_first: AtomicU32::new(0),
            retry_after_ms: None,
            latency: Duration::ZERO,
            calls: AtomicU32::new(0),
        }
    }

    /// The first `n` calls fail with a transient error.
    pub fn fail_first(self, n: u32) -> Self {
        self.fail_first.store(n, Ordering::SeqCst);
        self
    }

    /// Tell the runtime how long to wait before retrying.
    pub fn retry_after_ms(mut self, ms: u64) -> Self {
        self.retry_after_ms = Some(ms);
        self
    }

    /// Every call takes at least this long.
    pub fn latency(mut self, latency: Duration) -> Self {
        self.latency = latency;
        self
    }

    /// How many calls were made, failed ones included.
    pub fn calls(&self) -> u32 {
        self.calls.load(Ordering::SeqCst)
    }

    /// The wrapped model.
    pub fn inner(&self) -> &M {
        &self.inner
    }
}

#[async_trait]
impl<M: ModelGateway> ModelGateway for Faulty<M> {
    async fn generate(&self, request: ModelRequest, cancel: CancellationToken) -> Result<ModelResponse, ModelError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if !self.latency.is_zero() {
            tokio::select! {
                _ = tokio::time::sleep(self.latency) => {}
                _ = cancel.cancelled() => return Err(ModelError::Cancelled),
            }
        }
        // A compare-and-swap loop keeps this correct when calls overlap (and works on the oldest supported Rust).
        let failing = loop {
            let left = self.fail_first.load(Ordering::SeqCst);
            if left == 0 {
                break false;
            }
            if self.fail_first.compare_exchange(left, left - 1, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                break true;
            }
        };
        if failing {
            return Err(ModelError::Transient {
                message: "The simulated model is overloaded.".into(),
                retry_after_ms: self.retry_after_ms,
            });
        }
        self.inner.generate(request, cancel).await
    }
}

#[cfg(test)]
mod tests {
    use zorix_core::{ExecutionId, Message};

    use super::*;
    use crate::scripted::{ScriptedModel, Turn};

    fn request() -> ModelRequest {
        ModelRequest {
            model: "sim".into(),
            execution_id: ExecutionId::new(),
            messages: vec![Message::user("hi")],
            tools: vec![],
            response_schema: None,
        }
    }

    #[tokio::test]
    async fn the_first_calls_fail_then_the_inner_model_answers() {
        let model = Faulty::new(ScriptedModel::new([Turn::Say("ok".into())])).fail_first(2).retry_after_ms(5);
        for _ in 0..2 {
            let error = model.generate(request(), CancellationToken::new()).await.unwrap_err();
            assert_eq!(
                error,
                ModelError::Transient { message: "The simulated model is overloaded.".into(), retry_after_ms: Some(5) }
            );
        }
        assert_eq!(model.generate(request(), CancellationToken::new()).await.unwrap().text, "ok");
        assert_eq!(model.calls(), 3);
        assert_eq!(model.inner().remaining(), 0, "failed calls did not use up the script");
    }

    #[tokio::test]
    async fn latency_can_be_cancelled() {
        let model = Faulty::new(ScriptedModel::new([Turn::Say("ok".into())])).latency(Duration::from_secs(30));
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            stop.cancel();
        });
        assert_eq!(model.generate(request(), cancel).await, Err(ModelError::Cancelled));
    }
}
