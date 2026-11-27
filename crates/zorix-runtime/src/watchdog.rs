//! The time limit of one model call: the time it goes without showing progress, and a ceiling for the whole call.
//!
//! An answer that keeps arriving must not be cut because it is long, and one that has gone silent must not be waited for. Progress is
//! a piece of text reaching the live sink. A call that streams no text (a tool call that is still being written) shows none, so the
//! limit is, for it, the whole time it takes, as before.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::time::Instant;
use zorix_core::TextSink;

/// The most one model call may take in all, however steadily it answers. It matches the longest the providers' HTTP clients wait.
pub(crate) const MODEL_CALL_CEILING_MS: u64 = 600_000;

/// When a call last made progress, in milliseconds from when it started.
pub(crate) struct Progress {
    start: Instant,
    last_ms: AtomicU64,
}

impl Progress {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self { start: Instant::now(), last_ms: AtomicU64::new(0) })
    }

    fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn touch(&self) {
        self.last_ms.store(self.elapsed_ms(), Ordering::Relaxed);
    }
}

/// A sink that notes each piece of text as progress and passes it on.
pub(crate) struct Watched {
    inner: Arc<dyn TextSink>,
    progress: Arc<Progress>,
}

impl Watched {
    pub(crate) fn new(inner: Arc<dyn TextSink>, progress: Arc<Progress>) -> Arc<Self> {
        Arc::new(Self { inner, progress })
    }
}

impl TextSink for Watched {
    fn text(&self, delta: &str) {
        self.progress.touch();
        self.inner.text(delta);
    }

    fn reset(&self) {
        self.progress.touch();
        self.inner.reset();
    }
}

/// Which limit a call ran into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Timeout {
    /// `idle_ms` passed with no progress.
    NoProgress,
    /// The whole call took `ceiling_ms`.
    Ceiling,
}

/// Wait for `call`, but give up after `idle_ms` without progress or `ceiling_ms` in all. The ceiling is never lower than `idle_ms`.
pub(crate) async fn guard<F: Future>(
    progress: &Progress,
    idle_ms: u64,
    ceiling_ms: u64,
    call: F,
) -> Result<F::Output, Timeout> {
    let ceiling_ms = ceiling_ms.max(idle_ms);
    tokio::pin!(call);
    loop {
        let elapsed = progress.elapsed_ms();
        if elapsed >= ceiling_ms {
            return Err(Timeout::Ceiling);
        }
        let idle_left = progress.last_ms.load(Ordering::Relaxed).saturating_add(idle_ms).saturating_sub(elapsed);
        if idle_left == 0 {
            return Err(Timeout::NoProgress);
        }
        let wait = idle_left.min(ceiling_ms - elapsed);
        tokio::select! {
            out = &mut call => return Ok(out),
            _ = tokio::time::sleep(Duration::from_millis(wait)) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn progress_pushes_the_limit_back_and_the_ceiling_does_not_move() {
        let progress = Progress::new();
        let sink = Watched::new(Arc::new(zorix_core::NoSink), progress.clone());
        // Pieces every 80 ms for a second, with a 100 ms limit: never cut. Then silence: cut 100 ms after the last piece.
        let call = async {
            for _ in 0..12 {
                tokio::time::sleep(Duration::from_millis(80)).await;
                sink.text("x");
            }
            std::future::pending::<()>().await;
        };
        let started = Instant::now();
        let outcome = guard(&progress, 100, 10_000, call).await;
        assert_eq!(outcome, Err(Timeout::NoProgress));
        let took = started.elapsed().as_millis();
        assert!((1_050..=1_100).contains(&took), "{took} ms: 960 of pieces and 100 of silence");

        // A call that never goes quiet still ends at the ceiling.
        let progress = Progress::new();
        let sink = Watched::new(Arc::new(zorix_core::NoSink), progress.clone());
        let steady = async {
            loop {
                tokio::time::sleep(Duration::from_millis(50)).await;
                sink.text("x");
            }
        };
        assert_eq!(guard(&progress, 100, 2_000, steady).await, Err(Timeout::Ceiling));
    }

    #[tokio::test(start_paused = true)]
    async fn a_limit_that_cannot_be_reached_never_fires_and_a_finished_call_returns() {
        let progress = Progress::new();
        let call = async {
            tokio::time::sleep(Duration::from_secs(3_600)).await;
            7
        };
        // The workflow launcher passes the largest limit there is.
        assert_eq!(guard(&progress, u64::MAX, u64::MAX, call).await, Ok(7));
    }
}
