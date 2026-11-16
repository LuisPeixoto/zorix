//! Token calibration that survives restarts (ADR-0016).
//!
//! The rules live in `zorix_core::tokens`; this wrapper loads what was learned, saves what changes on a background thread
//! (so a model call never waits for the disk), and can seed itself from the estimates and usage already recorded in events.

use std::path::Path;
use std::sync::mpsc::{Sender, channel};

use rusqlite::{Connection, params};
use zorix_core::{Calibration, Error, InMemoryCalibration, Result, TokenCalibration, TokenSample};

/// How much a sample reconstructed from old events counts: those estimates left out the tool definitions.
pub const BACKFILL_WEIGHT: f64 = 0.5;
/// How many past model calls seed an empty table.
pub const BACKFILL_CALLS: usize = 1_000;

fn storage(error: impl std::fmt::Display) -> Error {
    Error::Storage(error.to_string())
}

/// A calibration kept in memory and written to the database as it changes.
pub struct SqliteCalibration {
    inner: InMemoryCalibration,
    writer: Sender<Vec<(String, Calibration)>>,
    path: std::path::PathBuf,
}

impl SqliteCalibration {
    /// Open the database at `path`, load what was learned and start the writer.
    pub fn open(path: &Path) -> Result<Self> {
        let mut conn = Connection::open(path).map_err(storage)?;
        conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(storage)?;
        crate::schema::migrate(&mut conn).map_err(Error::Storage)?;
        let known = {
            let mut stmt = conn.prepare("SELECT model, factor, samples FROM token_calibration").map_err(storage)?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, Calibration { factor: r.get(1)?, samples: r.get(2)? })))
                .map_err(storage)?;
            rows.collect::<rusqlite::Result<Vec<_>>>().map_err(storage)?
        };
        let (writer, receiver) = channel::<Vec<(String, Calibration)>>();
        std::thread::Builder::new()
            .name("zorix-calibration".into())
            .spawn(move || {
                // Ends when the last sender is dropped.
                for changes in receiver {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as i64)
                        .unwrap_or(0);
                    for (model, known) in changes {
                        let _ = conn.execute(
                            "INSERT INTO token_calibration (model, factor, samples, updated_at) VALUES (?1, ?2, ?3, ?4)
                             ON CONFLICT (model) DO UPDATE SET factor = excluded.factor, samples = excluded.samples, updated_at = excluded.updated_at",
                            params![model, known.factor, known.samples, now],
                        );
                    }
                }
            })
            .map_err(|e| Error::Internal(format!("could not start the calibration writer: {e}")))?;
        Ok(Self { inner: InMemoryCalibration::with(known), writer, path: path.to_path_buf() })
    }

    /// What is known about every model.
    pub fn all(&self) -> Vec<(String, Calibration)> {
        self.inner.all()
    }

    /// Seed an empty calibration from the model calls already recorded in events. Returns how many calls taught something.
    /// Does nothing when something has been learned already.
    pub fn backfill(&self) -> Result<usize> {
        if !self.inner.is_empty() {
            return Ok(0);
        }
        let conn = Connection::open(&self.path).map_err(storage)?;
        conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(storage)?;
        let mut stmt = conn
            .prepare(
                "SELECT q.payload, r.payload FROM events q
                 JOIN events r ON r.execution_id = q.execution_id AND r.kind = 'execution.model.responded'
                   AND json_extract(r.payload, '$.iteration') = json_extract(q.payload, '$.iteration')
                 WHERE q.kind = 'execution.model.requested' ORDER BY q.seq DESC LIMIT ?1",
            )
            .map_err(storage)?;
        let rows = stmt
            .query_map(params![BACKFILL_CALLS as i64], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(storage)?;
        let mut taught = 0;
        for row in rows {
            let (asked, answered) = row.map_err(storage)?;
            if let Some(sample) = sample_from_events(&asked, &answered) {
                let changed = self.inner.observe_weighted(&sample, BACKFILL_WEIGHT);
                if !changed.is_empty() {
                    taught += 1;
                    let _ = self.writer.send(changed);
                }
            }
        }
        Ok(taught)
    }
}

/// Rebuild a sample from a recorded `model.requested` and `model.responded` pair.
fn sample_from_events(requested: &str, responded: &str) -> Option<TokenSample> {
    let q: serde_json::Value = serde_json::from_str(requested).ok()?;
    let r: serde_json::Value = serde_json::from_str(responded).ok()?;
    let usage = &r["usage"];
    let count = |key: &str| usage[key].as_u64().unwrap_or(0);
    let provider = r["provider"].as_str();
    let model = r["model"].as_str();
    Some(TokenSample {
        requested: q["model"].as_str()?.to_string(),
        answered: provider.zip(model).map(|(p, m)| format!("{p}/{m}")),
        // Events written before 0.3 counted messages only.
        estimated: q["estimated_input_tokens"].as_u64()? + q["estimated_tool_tokens"].as_u64().unwrap_or(0),
        actual: count("input_tokens") + count("cache_read_tokens") + count("cache_write_tokens"),
        phase: if q["iteration"].as_u64() == Some(1) {
            zorix_core::StepPhase::Start
        } else {
            zorix_core::StepPhase::Continue
        },
    })
}

impl TokenCalibration for SqliteCalibration {
    fn factor(&self, model: &str) -> f64 {
        self.inner.factor(model)
    }

    fn observe(&self, sample: TokenSample) {
        let changed = self.inner.observe_weighted(&sample, 1.0);
        if !changed.is_empty() {
            let _ = self.writer.send(changed);
        }
    }
}
