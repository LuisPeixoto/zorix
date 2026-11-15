//! Backoff for retried model and tool calls.

use serde::{Deserialize, Serialize};

/// How long to wait between attempts: doubling, capped, with a little deterministic jitter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RetryPolicy {
    /// Wait before the second attempt, in milliseconds.
    pub base_delay_ms: u64,
    /// The longest wait, in milliseconds.
    pub max_delay_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self { base_delay_ms: 250, max_delay_ms: 8_000 }
    }
}

impl RetryPolicy {
    /// The wait after attempt number `attempt` (1 for the first) failed.
    ///
    /// `retry_after_ms` is what the provider asked for, if anything: it is respected (up to a minute).
    /// `seed` spreads out callers that fail together; the same seed always gives the same wait.
    pub fn delay_ms(&self, attempt: u32, retry_after_ms: Option<u64>, seed: u64) -> u64 {
        let doubled = self.base_delay_ms.saturating_mul(1u64 << attempt.saturating_sub(1).min(20));
        let capped = doubled.min(self.max_delay_ms);
        let mixed = (seed ^ u64::from(attempt)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let jitter = (mixed >> 33) % (capped / 4 + 1);
        let own = (capped + jitter).min(self.max_delay_ms.max(capped));
        match retry_after_ms {
            Some(asked) => own.max(asked.min(60_000)),
            None => own,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_grow_and_stop_at_the_cap() {
        let policy = RetryPolicy { base_delay_ms: 100, max_delay_ms: 1_000 };
        let waits: Vec<u64> = (1..=8).map(|n| policy.delay_ms(n, None, 7)).collect();
        assert!(waits[0] >= 100 && waits[0] <= 125);
        assert!(waits[1] >= 200 && waits[1] <= 250);
        assert!(waits.iter().all(|w| *w <= 1_000), "{waits:?}");
        assert_eq!(waits[7], 1_000, "capped (jitter cannot push past the cap)");
    }

    #[test]
    fn the_same_seed_gives_the_same_wait_and_different_seeds_spread() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.delay_ms(3, None, 42), policy.delay_ms(3, None, 42));
        let distinct: std::collections::HashSet<u64> = (0..50).map(|seed| policy.delay_ms(3, None, seed)).collect();
        assert!(distinct.len() > 5);
    }

    #[test]
    fn what_the_provider_asked_for_is_respected_within_reason() {
        let policy = RetryPolicy { base_delay_ms: 10, max_delay_ms: 100 };
        assert!(policy.delay_ms(1, Some(5_000), 1) >= 5_000);
        assert_eq!(policy.delay_ms(1, Some(10_000_000), 1), 60_000);
    }

    #[test]
    fn huge_attempt_numbers_do_not_overflow() {
        let policy = RetryPolicy::default();
        assert!(policy.delay_ms(u32::MAX, None, 0) <= policy.max_delay_ms);
    }
}
