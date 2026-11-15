//! Time. Everything reads the clock through [`Clock`] so tests can control it.

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// A point in time: milliseconds since the Unix epoch, in UTC.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(pub i64);

impl Timestamp {
    /// Milliseconds since the Unix epoch.
    pub fn millis(&self) -> i64 {
        self.0
    }

    /// This instant plus some milliseconds.
    pub fn plus_millis(&self, millis: u64) -> Timestamp {
        Timestamp(self.0.saturating_add(i64::try_from(millis).unwrap_or(i64::MAX)))
    }

    /// Milliseconds from `earlier` to this instant (zero if `earlier` is later).
    pub fn since(&self, earlier: Timestamp) -> u64 {
        u64::try_from(self.0.saturating_sub(earlier.0)).unwrap_or(0)
    }

    /// The instant as RFC 3339 text, such as `2026-11-15T18:44:56.123Z`.
    pub fn to_rfc3339(&self) -> String {
        let nanos = i128::from(self.0) * 1_000_000;
        OffsetDateTime::from_unix_timestamp_nanos(nanos)
            .ok()
            .and_then(|time| time.format(&Rfc3339).ok())
            .unwrap_or_else(|| format!("{}ms", self.0))
    }
}

/// A source of the current time.
pub trait Clock: Send + Sync {
    /// The current time.
    fn now(&self) -> Timestamp;
}

/// The computer's clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let millis = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        Timestamp(i64::try_from(millis).unwrap_or(i64::MAX))
    }
}

/// A clock that only moves when told to. For tests and simulations.
#[derive(Debug, Default)]
pub struct ManualClock(AtomicI64);

impl ManualClock {
    /// A clock that starts at `start`.
    pub fn new(start: Timestamp) -> Self {
        Self(AtomicI64::new(start.0))
    }

    /// Move the clock forward.
    pub fn advance(&self, millis: u64) {
        self.0.fetch_add(i64::try_from(millis).unwrap_or(i64::MAX), Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Timestamp {
        Timestamp(self.0.load(Ordering::SeqCst))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_as_rfc3339() {
        assert_eq!(Timestamp(0).to_rfc3339(), "1970-01-01T00:00:00Z");
        assert_eq!(Timestamp(1_700_000_000_123).to_rfc3339(), "2023-11-14T22:13:20.123Z");
    }

    #[test]
    fn the_manual_clock_moves_only_when_told() {
        let clock = ManualClock::new(Timestamp(1000));
        assert_eq!(clock.now(), Timestamp(1000));
        clock.advance(250);
        assert_eq!(clock.now(), Timestamp(1250));
        assert_eq!(clock.now().since(Timestamp(1000)), 250);
        assert_eq!(Timestamp(5).since(Timestamp(10)), 0);
    }

    #[test]
    fn the_system_clock_is_after_2025() {
        assert!(SystemClock.now() > Timestamp(1_735_689_600_000));
    }
}
