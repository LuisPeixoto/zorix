//! The SQLite implementation of [`OrgStore`] (ADR-0038). It opens its own connection like the other stores and every query carries
//! the tenant. A worker is written with its events in one transaction, and a worker's revision guards against lost updates.

use std::path::Path;
use std::sync::Arc;

use crate::errors::storage;
use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::{Connection as Db, OptionalExtension, Transaction, TransactionBehavior, params};
use zorix_core::{
    AgentSlug, Error, MAX_WORKERS, OrgEvent, OrgEventDraft, OrgReport, OrgReportId, OrgSettings, OrgStore,
    ReportStatus, Result, TenantId, Worker,
};

/// Stores the workforce in the same SQLite file as everything else.
#[derive(Clone)]
pub struct SqliteOrgStore {
    conn: Arc<Mutex<Db>>,
}

fn invalid(what: impl Into<String>) -> Error {
    Error::InvalidInput(what.into())
}

fn parse<T: serde::de::DeserializeOwned>(text: &str) -> Result<T> {
    serde_json::from_str(text).map_err(|e| Error::Storage(format!("a stored record could not be read: {e}")))
}

fn encode<T: serde::Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(storage)
}

impl SqliteOrgStore {
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

    fn with<T>(&self, work: impl FnOnce(&mut Db) -> Result<T>) -> Result<T> {
        let mut conn = self.conn.lock();
        work(&mut conn)
    }
}

fn build_worker(raw: (i64, String)) -> Result<Worker> {
    let mut worker: Worker = parse(&raw.1)?;
    worker.revision = u64::try_from(raw.0).unwrap_or(0);
    Ok(worker)
}

fn worker_from(row: &rusqlite::Row<'_>) -> rusqlite::Result<(i64, String)> {
    Ok((row.get(0)?, row.get(1)?))
}

fn load_worker(db: &Db, tenant: &TenantId, agent: &AgentSlug) -> Result<Option<Worker>> {
    db.query_row(
        "SELECT revision, doc FROM workers WHERE tenant = ?1 AND agent = ?2",
        params![tenant.as_str(), agent.as_str()],
        worker_from,
    )
    .optional()
    .map_err(storage)?
    .map(build_worker)
    .transpose()
}

/// Number and store events after the highest number the tenant has.
fn insert_events(tx: &Transaction<'_>, tenant: &TenantId, drafts: &[OrgEventDraft]) -> Result<()> {
    if drafts.is_empty() {
        return Ok(());
    }
    let mut seq: u64 = tx
        .query_row("SELECT COALESCE(MAX(seq), 0) FROM org_events WHERE tenant = ?1", [tenant.as_str()], |r| {
            r.get::<_, i64>(0)
        })
        .map_err(storage)
        .map(|n| u64::try_from(n).unwrap_or(0))?;
    for draft in drafts {
        seq += 1;
        let event = OrgEvent {
            seq,
            kind: draft.kind,
            worker: draft.worker.clone(),
            text: draft.text.clone(),
            by: draft.by.clone(),
            run: draft.run,
            at: draft.at,
        };
        tx.execute(
            "INSERT INTO org_events (tenant, seq, worker, at, doc) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                tenant.as_str(),
                i64::try_from(seq).map_err(storage)?,
                draft.worker.as_ref().map(|w| w.as_str().to_string()),
                draft.at.millis(),
                encode(&event)?
            ],
        )
        .map_err(storage)?;
    }
    Ok(())
}

fn status_text(status: ReportStatus) -> &'static str {
    match status {
        ReportStatus::Running => "running",
        ReportStatus::Done => "done",
        ReportStatus::Failed => "failed",
    }
}

#[async_trait]
impl OrgStore for SqliteOrgStore {
    async fn settings(&self, tenant: &TenantId) -> Result<OrgSettings> {
        self.with(|db| {
            let doc: Option<String> = db
                .query_row("SELECT doc FROM org_settings WHERE tenant = ?1", [tenant.as_str()], |r| r.get(0))
                .optional()
                .map_err(storage)?;
            match doc {
                Some(doc) => parse(&doc),
                None => Ok(OrgSettings::default()),
            }
        })
    }

    async fn put_settings(&self, tenant: &TenantId, settings: &OrgSettings, events: &[OrgEventDraft]) -> Result<()> {
        settings.validate()?;
        self.with(|db| {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
            tx.execute(
                "INSERT INTO org_settings (tenant, doc) VALUES (?1, ?2) ON CONFLICT(tenant) DO UPDATE SET doc = excluded.doc",
                params![tenant.as_str(), encode(settings)?],
            )
            .map_err(storage)?;
            insert_events(&tx, tenant, events)?;
            tx.commit().map_err(storage)
        })
    }

    async fn workers(&self, tenant: &TenantId) -> Result<Vec<Worker>> {
        self.with(|db| {
            let mut statement = db
                .prepare("SELECT revision, doc FROM workers WHERE tenant = ?1 ORDER BY hired_at, agent")
                .map_err(storage)?;
            let rows = statement
                .query_map([tenant.as_str()], worker_from)
                .map_err(storage)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(storage)?;
            rows.into_iter().map(build_worker).collect()
        })
    }

    async fn worker(&self, tenant: &TenantId, agent: &AgentSlug) -> Result<Option<Worker>> {
        self.with(|db| load_worker(db, tenant, agent))
    }

    async fn put_worker(
        &self,
        tenant: &TenantId,
        worker: &Worker,
        expected: Option<u64>,
        events: &[OrgEventDraft],
    ) -> Result<bool> {
        worker.validate()?;
        self.with(|db| {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
            let written = match expected {
                None => {
                    let count: i64 = tx
                        .query_row("SELECT COUNT(*) FROM workers WHERE tenant = ?1", [tenant.as_str()], |r| r.get(0))
                        .map_err(storage)?;
                    if count >= MAX_WORKERS as i64 {
                        return Err(invalid(format!("A tenant keeps at most {MAX_WORKERS} workers.")));
                    }
                    if load_worker(&tx, tenant, &worker.agent)?.is_some() {
                        return Err(invalid(format!("{} is already a worker.", worker.agent)));
                    }
                    tx.execute(
                        "INSERT INTO workers (tenant, agent, revision, hired_at, doc) VALUES (?1, ?2, 1, ?3, ?4)",
                        params![tenant.as_str(), worker.agent.as_str(), worker.hired_at.millis(), encode(worker)?],
                    )
                    .map_err(storage)?;
                    true
                }
                Some(revision) => {
                    let changed = tx
                        .execute(
                            "UPDATE workers SET revision = revision + 1, doc = ?1 WHERE tenant = ?2 AND agent = ?3 AND revision = ?4",
                            params![
                                encode(worker)?,
                                tenant.as_str(),
                                worker.agent.as_str(),
                                i64::try_from(revision).map_err(storage)?
                            ],
                        )
                        .map_err(storage)?;
                    changed > 0
                }
            };
            if written {
                insert_events(&tx, tenant, events)?;
            }
            tx.commit().map_err(storage)?;
            Ok(written)
        })
    }

    async fn remove_worker(&self, tenant: &TenantId, agent: &AgentSlug, events: &[OrgEventDraft]) -> Result<bool> {
        self.with(|db| {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
            let removed = tx
                .execute(
                    "DELETE FROM workers WHERE tenant = ?1 AND agent = ?2",
                    params![tenant.as_str(), agent.as_str()],
                )
                .map_err(storage)?;
            if removed > 0 {
                insert_events(&tx, tenant, events)?;
            }
            tx.commit().map_err(storage)?;
            Ok(removed > 0)
        })
    }

    async fn add_report(&self, tenant: &TenantId, report: &OrgReport) -> Result<()> {
        self.with(|db| {
            db.execute(
                "INSERT INTO org_reports (id, tenant, worker, status, started_at, doc) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    report.id.to_string(),
                    tenant.as_str(),
                    report.worker.as_str(),
                    status_text(report.status),
                    report.started_at.millis(),
                    encode(report)?
                ],
            )
            .map_err(storage)?;
            Ok(())
        })
    }

    async fn update_report(&self, tenant: &TenantId, report: &OrgReport, events: &[OrgEventDraft]) -> Result<()> {
        self.with(|db| {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
            let changed = tx
                .execute(
                    "UPDATE org_reports SET status = ?1, doc = ?2 WHERE id = ?3 AND tenant = ?4",
                    params![status_text(report.status), encode(report)?, report.id.to_string(), tenant.as_str()],
                )
                .map_err(storage)?;
            if changed == 0 {
                return Err(Error::Missing(format!("report {}", report.id)));
            }
            insert_events(&tx, tenant, events)?;
            tx.commit().map_err(storage)
        })
    }

    async fn report(&self, tenant: &TenantId, id: OrgReportId) -> Result<Option<OrgReport>> {
        self.with(|db| {
            db.query_row(
                "SELECT doc FROM org_reports WHERE id = ?1 AND tenant = ?2",
                params![id.to_string(), tenant.as_str()],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(storage)?
            .map(|doc| parse(&doc))
            .transpose()
        })
    }

    async fn reports(
        &self,
        tenant: &TenantId,
        worker: Option<&AgentSlug>,
        running_only: bool,
        limit: usize,
    ) -> Result<Vec<OrgReport>> {
        self.with(|db| {
            let mut sql = String::from("SELECT doc FROM org_reports WHERE tenant = ?1");
            let mut args: Vec<String> = vec![tenant.as_str().to_string()];
            if let Some(worker) = worker {
                sql.push_str(" AND worker = ?");
                args.push(worker.as_str().to_string());
            }
            if running_only {
                sql.push_str(" AND status = 'running'");
            }
            sql.push_str(" ORDER BY started_at DESC, id DESC LIMIT ?");
            let limit = i64::try_from(if limit == 0 { 50 } else { limit }).map_err(storage)?;
            let mut statement = db.prepare(&sql).map_err(storage)?;
            let mut bound: Vec<&dyn rusqlite::ToSql> = args.iter().map(|a| a as &dyn rusqlite::ToSql).collect();
            bound.push(&limit);
            let rows = statement
                .query_map(rusqlite::params_from_iter(bound), |r| r.get::<_, String>(0))
                .map_err(storage)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(storage)?;
            rows.iter().map(|doc| parse(doc)).collect()
        })
    }

    async fn add_events(&self, tenant: &TenantId, events: &[OrgEventDraft]) -> Result<()> {
        self.with(|db| {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
            insert_events(&tx, tenant, events)?;
            tx.commit().map_err(storage)
        })
    }

    async fn events(&self, tenant: &TenantId, worker: Option<&AgentSlug>, limit: usize) -> Result<Vec<OrgEvent>> {
        self.with(|db| {
            let mut sql = String::from("SELECT doc FROM org_events WHERE tenant = ?1");
            let mut args: Vec<String> = vec![tenant.as_str().to_string()];
            if let Some(worker) = worker {
                sql.push_str(" AND worker = ?");
                args.push(worker.as_str().to_string());
            }
            sql.push_str(" ORDER BY seq DESC LIMIT ?");
            let limit = i64::try_from(if limit == 0 { 100 } else { limit }).map_err(storage)?;
            let mut statement = db.prepare(&sql).map_err(storage)?;
            let mut bound: Vec<&dyn rusqlite::ToSql> = args.iter().map(|a| a as &dyn rusqlite::ToSql).collect();
            bound.push(&limit);
            let rows = statement
                .query_map(rusqlite::params_from_iter(bound), |r| r.get::<_, String>(0))
                .map_err(storage)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(storage)?;
            rows.iter().map(|doc| parse(doc)).collect()
        })
    }
}
