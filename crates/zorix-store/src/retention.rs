//! Retention (ADR-0044): what happens to a finished run that is old enough.
//!
//! A run is eligible when it and every run in its tree has finished and is older than the cutoff. *Compact* removes what is bulky (events,
//! checkpoints, tool calls, approvals, the recall snapshot) and keeps the run itself with its request, its answer, its usage, its cost and its
//! times, so lists, spend figures and budgets stay as they were. *Delete* removes the run too, unless a task, a workflow run, a report or an
//! organisation event refers to it, in which case it is compacted instead.
//!
//! Work is done in small transactions so it never holds the database for long, and `preview` counts without changing anything.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use crate::errors::storage;
use parking_lot::Mutex;
use rusqlite::{Connection as Db, params};
use tokio_util::sync::CancellationToken;
use zorix_core::{
    AppSettings, Clock, Error, ExecutionId, Result, RetentionMode, RetentionReport, SettingsCell, TenantId, Timestamp,
    operation_key,
};

/// How many runs one transaction handles.
const BATCH: usize = 200;

/// Applies retention to the same SQLite file as everything else.
#[derive(Clone)]
pub struct SqliteRetention {
    conn: Arc<Mutex<Db>>,
}

impl SqliteRetention {
    /// Open the database at `path` (creating and migrating it when needed) with its own connection.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Storage(format!("could not create {}: {e}", parent.display())))?;
        }
        Self::prepare(Db::open(path).map_err(storage)?, true)
    }

    /// A database in memory, for tests.
    pub fn open_in_memory() -> Result<Self> {
        Self::prepare(Db::open_in_memory().map_err(storage)?, false)
    }

    fn prepare(mut conn: Db, file: bool) -> Result<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(storage)?;
        if file {
            conn.pragma_update(None, "journal_mode", "WAL").map_err(storage)?;
            conn.pragma_update(None, "synchronous", "NORMAL").map_err(storage)?;
        }
        conn.pragma_update(None, "secure_delete", "ON").map_err(storage)?;
        crate::schema::migrate(&mut conn).map_err(Error::Storage)?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    /// One pass for the saved settings: `None` when retention is off. `now` is when it is, so the cutoff is `days` before it.
    pub async fn pass(
        &self,
        tenant: &TenantId,
        settings: &AppSettings,
        now: Timestamp,
        dry_run: bool,
    ) -> Result<Option<RetentionReport>> {
        let Some(days) = settings.retention.days else { return Ok(None) };
        let cutoff = Timestamp(now.millis().saturating_sub(i64::from(days) * 86_400_000));
        let mode = settings.retention.mode;
        Ok(Some(if dry_run {
            self.preview(tenant, cutoff, mode).await?
        } else {
            self.apply(tenant, cutoff, mode).await?
        }))
    }

    /// Run a pass ten minutes after the program starts and then once a day, for as long as the program runs. It reads the settings each time, so
    /// turning retention on or off needs no restart, and a pass that fails is logged and tried again the next day.
    pub fn run_daily(
        &self,
        tenant: TenantId,
        settings: SettingsCell,
        clock: Arc<dyn Clock>,
        stop: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        let retention = self.clone();
        tokio::spawn(async move {
            let mut wait = std::time::Duration::from_secs(600);
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(wait) => {}
                    _ = stop.cancelled() => return,
                }
                wait = std::time::Duration::from_secs(86_400);
                match retention.pass(&tenant, &settings.get(), clock.now(), false).await {
                    Ok(Some(report)) if report.runs > 0 => tracing::info!(
                        runs = report.runs,
                        deleted = report.deleted,
                        compacted = report.compacted,
                        bytes = report.bytes,
                        "retention made room"
                    ),
                    Ok(_) => {}
                    Err(error) => tracing::warn!(%error, "retention could not run; it tries again tomorrow"),
                }
            }
        })
    }

    /// What `apply` would do with the same arguments, without changing anything.
    pub async fn preview(&self, tenant: &TenantId, cutoff: Timestamp, mode: RetentionMode) -> Result<RetentionReport> {
        self.work(tenant, cutoff, mode, true).await
    }

    /// Compact or delete the runs of `tenant` that finished before `cutoff`.
    pub async fn apply(&self, tenant: &TenantId, cutoff: Timestamp, mode: RetentionMode) -> Result<RetentionReport> {
        self.work(tenant, cutoff, mode, false).await
    }

    async fn work(
        &self,
        tenant: &TenantId,
        cutoff: Timestamp,
        mode: RetentionMode,
        dry_run: bool,
    ) -> Result<RetentionReport> {
        let (conn, tenant) = (self.conn.clone(), tenant.clone());
        tokio::task::spawn_blocking(move || run(&mut conn.lock(), &tenant, cutoff, mode, dry_run))
            .await
            .map_err(|e| Error::Internal(format!("a storage task failed: {e}")))?
    }
}

/// The runs that may go: finished, before the cutoff, and alone in no tree that has something still moving or newer.
fn eligible(db: &Db, tenant: &TenantId, cutoff: Timestamp) -> Result<Vec<String>> {
    // (id, tree, finished before the cutoff and terminal)
    let mut stmt = db
        .prepare(
            "SELECT id, COALESCE(root, id), state, COALESCE(json_extract(doc, '$.finished_at'), created_at) FROM executions
             WHERE tenant = ?1 AND compacted = 0 ORDER BY created_at, id",
        )
        .map_err(storage)?;
    let rows = stmt
        .query_map(params![tenant.as_str()], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?))
        })
        .map_err(storage)?;
    let mut trees: HashMap<String, (Vec<String>, bool)> = HashMap::new();
    for row in rows {
        let (id, tree, state, finished) = row.map_err(storage)?;
        let old_and_done = matches!(state.as_str(), "completed" | "failed" | "cancelled") && finished < cutoff.millis();
        let entry = trees.entry(tree).or_insert_with(|| (Vec::new(), true));
        entry.0.push(id);
        entry.1 &= old_and_done;
    }
    let mut ids: Vec<String> = trees.into_values().filter(|(_, ok)| *ok).flat_map(|(ids, _)| ids).collect();
    ids.sort();
    Ok(ids)
}

/// Every id of an execution that a record of another kind mentions.
fn referenced(db: &Db) -> Result<HashSet<String>> {
    let mut found = HashSet::new();
    for sql in [
        "SELECT doc FROM tasks",
        "SELECT doc FROM project_events",
        "SELECT doc FROM workflow_runs",
        "SELECT doc FROM org_reports",
        "SELECT doc FROM org_events",
    ] {
        let mut stmt = db.prepare(sql).map_err(storage)?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(storage)?;
        for doc in rows {
            let doc = doc.map_err(storage)?;
            found.extend(uuids_in(&doc));
        }
    }
    Ok(found)
}

/// The text of every `8-4-4-4-12` hexadecimal id in `text`.
fn uuids_in(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut at = 0;
    while at + 36 <= bytes.len() {
        let window = &bytes[at..at + 36];
        let fits = window.iter().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => *b == b'-',
            _ => b.is_ascii_hexdigit(),
        });
        if fits {
            found.push(text[at..at + 36].to_ascii_lowercase());
            at += 36;
        } else {
            at += 1;
        }
    }
    found
}

fn count(db: &Db, sql: &str, ids: &[String]) -> Result<(u64, u64)> {
    // Counts and bytes for the rows of these executions.
    let marks = vec!["?"; ids.len()].join(",");
    let sql = sql.replace("{ids}", &marks);
    let (n, bytes): (i64, i64) =
        db.query_row(&sql, rusqlite::params_from_iter(ids.iter()), |r| Ok((r.get(0)?, r.get(1)?))).map_err(storage)?;
    Ok((n as u64, bytes as u64))
}

fn add_counts(db: &Db, ids: &[String], report: &mut RetentionReport) -> Result<()> {
    let (n, b) =
        count(db, "SELECT COUNT(*), COALESCE(SUM(length(payload)), 0) FROM events WHERE execution_id IN ({ids})", ids)?;
    report.events += n;
    report.bytes += b;
    let (n, b) = count(
        db,
        "SELECT COUNT(*), COALESCE(SUM(length(doc)), 0) FROM checkpoints WHERE execution_id IN ({ids})",
        ids,
    )?;
    report.checkpoints += n;
    report.bytes += b;
    let (n, b) =
        count(db, "SELECT COUNT(*), COALESCE(SUM(length(doc)), 0) FROM tool_calls WHERE execution_id IN ({ids})", ids)?;
    report.tool_calls += n;
    report.bytes += b;
    let (n, b) =
        count(db, "SELECT COUNT(*), COALESCE(SUM(length(doc)), 0) FROM approvals WHERE execution_id IN ({ids})", ids)?;
    report.approvals += n;
    report.bytes += b;
    let (n, b) = count(
        db,
        "SELECT COUNT(*), COALESCE(SUM(length(doc)), 0) FROM context_snapshots WHERE execution_id IN ({ids})",
        ids,
    )?;
    report.snapshots += n;
    report.bytes += b;
    Ok(())
}

fn run(
    db: &mut Db,
    tenant: &TenantId,
    cutoff: Timestamp,
    mode: RetentionMode,
    dry_run: bool,
) -> Result<RetentionReport> {
    let mut report = RetentionReport::default();
    let ids = eligible(db, tenant, cutoff)?;
    if ids.is_empty() {
        return Ok(report);
    }
    let refer = if mode == RetentionMode::Delete { referenced(db)? } else { HashSet::new() };
    for batch in ids.chunks(BATCH) {
        add_counts(db, batch, &mut report)?;
        for id in batch {
            report.runs += 1;
            if mode == RetentionMode::Delete && !refer.contains(&id.to_ascii_lowercase()) {
                report.deleted += 1;
            } else {
                report.compacted += 1;
                if mode == RetentionMode::Delete {
                    report.kept_because_referenced += 1;
                }
            }
        }
        if dry_run {
            continue;
        }
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(storage)?;
        for id in batch {
            for table in
                ["events", "checkpoints", "tool_calls", "approvals", "context_snapshots", "snapshot_items", "leases"]
            {
                tx.execute(&format!("DELETE FROM {table} WHERE execution_id = ?1"), params![id]).map_err(storage)?;
            }
            if mode == RetentionMode::Delete && !refer.contains(&id.to_ascii_lowercase()) {
                // The record of what the run did (its past work) goes with it, and so does its text in other runs' frozen recall.
                if let Ok(execution) = id.parse::<ExecutionId>() {
                    crate::memory::scrub(&tx, &[operation_key(execution)])?;
                }
                tx.execute("DELETE FROM operations WHERE execution_id = ?1", params![id]).map_err(storage)?;
                tx.execute("DELETE FROM executions WHERE id = ?1", params![id]).map_err(storage)?;
            } else {
                tx.execute(
                    "UPDATE executions SET compacted = 1, doc = json_set(doc, '$.compacted', json('true')) WHERE id = ?1",
                    params![id],
                )
                .map_err(storage)?;
            }
        }
        tx.commit().map_err(storage)?;
    }
    if !dry_run {
        // The freed pages and the log forget what was removed (secure_delete overwrote it).
        let _ = db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_found_in_a_record_whatever_surrounds_them() {
        let id = "0192f3a4-7b1c-7d2e-8f3a-4b5c6d7e8f90";
        let doc =
            format!(r#"{{"run":"{id}","also":["{}", "not-an-id", "0192f3a4-7b1c-7d2e-8f3a"]}}"#, id.to_uppercase());
        assert_eq!(uuids_in(&doc), vec![id.to_string(), id.to_string()]);
        assert!(uuids_in("").is_empty());
        assert!(uuids_in("0192f3a4-7b1c-7d2e-8f3a-4b5c6d7e8f9").is_empty(), "one character short");
    }
}
