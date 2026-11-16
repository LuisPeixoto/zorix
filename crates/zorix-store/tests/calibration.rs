//! Calibration kept in the database: it survives a restart, never blocks, and can be seeded from past events.

use std::time::Duration;

use zorix_core::{
    Execution, ExecutionId, ExecutionRepository, NewExecution, StepPhase, TaskRequest, Timestamp, TokenCalibration,
    TokenSample,
};
use zorix_store::{SqliteCalibration, SqliteRepository};

fn sample(estimated: u64, actual: u64) -> TokenSample {
    TokenSample { requested: "anthropic/claude".into(), answered: None, estimated, actual, phase: StepPhase::Start }
}

/// Wait for the writer thread to save (it is asynchronous by design).
fn eventually(mut check: impl FnMut() -> bool) -> bool {
    for _ in 0..100 {
        if check() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn what_was_learned_is_still_known_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zorix.sqlite");
    let first = SqliteCalibration::open(&path).unwrap();
    for _ in 0..4 {
        first.observe(sample(1_000, 1_300));
    }
    assert!((first.factor("anthropic/claude") - 1.3).abs() < 1e-9);
    drop(first);
    let reopened = loop_until_loaded(&path, "anthropic/claude");
    assert!((reopened.factor("anthropic/claude") - 1.3).abs() < 1e-9);
    assert_eq!(reopened.all().len(), 1);
    assert_eq!(reopened.all()[0].1.samples, 4);
}

fn loop_until_loaded(path: &std::path::Path, model: &str) -> SqliteCalibration {
    let mut loaded = None;
    assert!(
        eventually(|| {
            let c = SqliteCalibration::open(path).unwrap();
            let ok = c.all().iter().any(|(m, k)| m == model && k.samples == 4);
            loaded = Some(c);
            ok
        }),
        "the writer saved what it learned"
    );
    loaded.unwrap()
}

#[tokio::test]
async fn an_empty_calibration_is_seeded_from_past_events_at_half_weight() {
    use zorix_core::{EventKind, NewEvent};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zorix.sqlite");
    let repository = SqliteRepository::open(&path).unwrap();
    let new = NewExecution {
        request: TaskRequest { prompt: "p".into(), ..TaskRequest::default() },
        ..NewExecution::default()
    };
    let execution = Execution::new(ExecutionId::new(), new, Timestamp(1));
    let mut events = Vec::new();
    for iteration in 1..=4u64 {
        events.push(NewEvent {
            at: Timestamp(iteration as i64),
            kind: EventKind::ModelRequested,
            payload: serde_json::json!({"iteration": iteration, "model": "anthropic/claude", "estimated_input_tokens": 1_000}),
        });
        events.push(NewEvent {
            at: Timestamp(iteration as i64),
            kind: EventKind::ModelResponded,
            payload: serde_json::json!({"iteration": iteration, "model": "claude", "provider": "anthropic",
                "usage": {"input_tokens": 400, "cache_read_tokens": 1_600, "cache_write_tokens": 0, "output_tokens": 9}}),
        });
    }
    repository.create(execution, events).await.unwrap();
    let calibration = SqliteCalibration::open(&path).unwrap();
    assert_eq!(calibration.backfill().unwrap(), 4);
    let factor = calibration.factor("anthropic/claude");
    assert!(factor > 1.0 && factor < 2.0, "the real ratio is 2.0, but old evidence counts for half: {factor}");
    // Seeding again does nothing once something is known.
    assert_eq!(calibration.backfill().unwrap(), 0);
}

#[test]
fn an_empty_database_has_nothing_to_seed_from() {
    let dir = tempfile::tempdir().unwrap();
    let calibration = SqliteCalibration::open(&dir.path().join("zorix.sqlite")).unwrap();
    assert_eq!(calibration.backfill().unwrap(), 0);
    assert!(calibration.all().is_empty());
    assert_eq!(calibration.factor("anything"), 1.0);
}
