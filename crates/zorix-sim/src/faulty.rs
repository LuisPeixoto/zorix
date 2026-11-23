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
    error: Option<ModelError>,
    seeded: Option<(u64, u32)>,
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
            error: None,
            seeded: None,
        }
    }

    /// The error a failing call gives, in place of the default temporary one (a rate limit, a refusal, bad credentials...).
    pub fn fail_with(mut self, error: ModelError) -> Self {
        self.error = Some(error);
        self
    }

    /// From a seed, fail about `percent` calls in a hundred. The same seed fails the same calls in the same order, so a run that
    /// found a problem can be run again. The first call is never one of them, so a run always starts.
    pub fn seeded(mut self, seed: u64, percent: u32) -> Self {
        self.seeded = Some((seed, percent.min(100)));
        self
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

/// A small, fixed mixing function (splitmix64), so the faults of a seed do not depend on a library's version.
fn mix(seed: u64, call: u64) -> u64 {
    let mut z = seed.wrapping_add(call.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
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
        let call = u64::from(self.calls.load(Ordering::SeqCst));
        let by_seed = self.seeded.is_some_and(|(seed, percent)| call > 1 && mix(seed, call) % 100 < u64::from(percent));
        let by_count = loop {
            let left = self.fail_first.load(Ordering::SeqCst);
            if left == 0 {
                break false;
            }
            if self.fail_first.compare_exchange(left, left - 1, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                break true;
            }
        };
        if by_count || by_seed {
            return Err(self.error.clone().unwrap_or_else(|| ModelError::Transient {
                message: "The simulated model is overloaded.".into(),
                retry_after_ms: self.retry_after_ms,
            }));
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
            ..ModelRequest::default()
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
    async fn a_failing_call_can_carry_any_error() {
        let error = ModelError::Provider {
            kind: zorix_core::ModelErrorKind::RateLimited,
            message: "Slow down.".into(),
            retry_after_ms: Some(7),
        };
        let model = Faulty::new(ScriptedModel::new([Turn::Say("ok".into())])).fail_first(1).fail_with(error.clone());
        assert_eq!(model.generate(request(), CancellationToken::new()).await.unwrap_err(), error);
    }

    #[tokio::test]
    async fn the_same_seed_fails_the_same_calls_and_the_first_call_always_works() {
        async fn pattern(seed: u64, percent: u32) -> Vec<bool> {
            let model = Faulty::new(ScriptedModel::new((0..200).map(|_| Turn::Say("ok".into())))).seeded(seed, percent);
            let mut out = Vec::new();
            for _ in 0..200 {
                out.push(model.generate(request(), CancellationToken::new()).await.is_err());
            }
            out
        }
        let a = pattern(42, 30).await;
        assert_eq!(a, pattern(42, 30).await, "repeatable");
        assert_ne!(a, pattern(43, 30).await, "another seed, another pattern");
        assert!(!a[0], "the first call works");
        let failed = a.iter().filter(|f| **f).count();
        assert!((30..=90).contains(&failed), "about 30 % of 200, got {failed}");
        assert!(pattern(7, 0).await.iter().all(|f| !f));
        assert!(pattern(7, 100).await.iter().skip(1).all(|f| *f));
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
