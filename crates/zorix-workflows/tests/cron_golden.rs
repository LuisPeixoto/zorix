//! The answers the previous cron library gave, kept as they were (ADR-0041): for each expression, zone and starting moment, the next
//! fires as seconds after the start. The evaluator that replaced it must give the same ones. A form it refuses on purpose is listed
//! here with the reason, and nothing else may differ.

use zorix_core::{Timestamp, When};
use zorix_workflows::schedule::next_fire;

/// Forms the old library accepted that were never documented, and that ZORIX now refuses with a clear message.
const REFUSED_ON_PURPOSE: &[&str] = &[
    "0 0 L * *",     // the last day of the month
    "0 0 * * 5L",    // the last Friday
    "0 0 * * 1#1",   // the first Monday
    "0 0 * * mon#2", // the second Monday
    "0 0 15W * *",   // the weekday nearest the 15th
    "0 0 ? * 1",     // "no specific value"
    "0 0 ,1 * *",    // an empty item in a list
];

#[test]
fn the_new_evaluator_gives_the_answers_the_old_library_gave() {
    let golden: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/cron_golden.json")).expect("the fixture is JSON");
    assert!(golden.len() > 2_200, "{} cases", golden.len());
    let (mut compared, mut refused, mut skipped) = (0, 0, 0);
    let mut differences = Vec::new();
    for entry in &golden {
        let (expr, zone, start) =
            (entry["expr"].as_str().unwrap(), entry["zone"].as_str().unwrap(), entry["start"].as_i64().unwrap());
        let when = When::Cron { expr: expr.to_string() };
        let ours: Result<Vec<i64>, String> = (|| {
            let mut at = Timestamp(start);
            let mut out = Vec::new();
            for _ in 0..14 {
                match next_fire(&when, Some(zone), at)? {
                    Some(next) => {
                        out.push((next.millis() - start) / 1000);
                        at = next;
                    }
                    None => break,
                }
            }
            Ok(out)
        })();
        if entry["fires"].is_null() {
            // The old library refused it: so do we.
            assert!(ours.is_err(), "{expr:?} was refused before and is now accepted: {ours:?}");
            refused += 1;
            continue;
        }
        if REFUSED_ON_PURPOSE.contains(&expr) {
            assert!(ours.is_err(), "{expr:?} is meant to be refused");
            skipped += 1;
            continue;
        }
        let golden_fires: Vec<i64> = entry["fires"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
        match ours {
            Ok(fires) if fires == golden_fires => compared += 1,
            other => differences.push(format!("{expr:?} in {zone} from {start}: old {golden_fires:?}, new {other:?}")),
        }
    }
    assert!(
        differences.is_empty(),
        "{} differences, the first of them:\n{}",
        differences.len(),
        differences.iter().take(6).cloned().collect::<Vec<_>>().join("\n")
    );
    assert!(
        compared >= 1_800 && refused >= 250 && skipped >= 100,
        "compared {compared}, refused {refused}, skipped {skipped}"
    );
}
