//! Schedules: any cron expression and any time zone name, no panic, and a next fire is always after the moment asked from.
#![no_main]
use libfuzzer_sys::fuzz_target;
use zorix_core::{Timestamp, When};
use zorix_workflows::schedule::next_fire;

fuzz_target!(|input: (&str, &str, i64)| {
    let (expr, zone, at) = input;
    let at = Timestamp(at.rem_euclid(4_102_444_800_000));
    let when = When::Cron { expr: expr.to_string() };
    if let Ok(Some(next)) = next_fire(&when, Some(zone), at) {
        assert!(next > at, "{expr:?} in {zone:?}: {next:?} is not after {at:?}");
    }
});
