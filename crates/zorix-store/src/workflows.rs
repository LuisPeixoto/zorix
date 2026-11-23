//! The SQLite implementation of [`WorkflowStore`] (ADR-0033 to ADR-0035). It opens its own connection like the other stores
//! and every query carries the tenant. A published version is never changed after it is written.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::{Connection as Db, OptionalExtension, params};
use zorix_core::workflow::{MAX_WORKFLOW_VERSIONS, MAX_WORKFLOWS};
use zorix_core::{
    Confirm, Confirmation, Error, Result, TenantId, Timestamp, TriggerState, WaitDecision, WaitId, Workflow,
    WorkflowId, WorkflowRun, WorkflowRunId, WorkflowSlug, WorkflowSpec, WorkflowStore, WorkflowVersion, WorkflowWait,
};

/// Stores workflows in the same SQLite file as everything else.
#[derive(Clone)]
pub struct SqliteWorkflowStore {
    conn: Arc<Mutex<Db>>,
}

fn storage(error: impl std::fmt::Display) -> Error {
    Error::Storage(error.to_string())
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

impl SqliteWorkflowStore {
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

type RawWorkflow = (String, String, String, bool, Option<u32>, i64, i64, String);

const WORKFLOW_COLUMNS: &str = "id, tenant, slug, enabled, live_version, created_at, updated_at, doc";

fn workflow_from(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawWorkflow> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?))
}

fn build_workflow(raw: RawWorkflow) -> Result<Workflow> {
    let (id, tenant, slug, enabled, live, created, updated, doc) = raw;
    Ok(Workflow {
        id: id.parse().map_err(storage)?,
        tenant: TenantId::new(tenant).map_err(storage)?,
        slug: WorkflowSlug::new(slug).map_err(storage)?,
        draft: parse::<WorkflowSpec>(&doc)?,
        live_version: live,
        enabled,
        created_at: Timestamp(created),
        updated_at: Timestamp(updated),
    })
}

fn load_workflow(db: &Db, tenant: &TenantId, id: WorkflowId) -> Result<Option<Workflow>> {
    db.query_row(
        &format!("SELECT {WORKFLOW_COLUMNS} FROM workflows WHERE id = ?1 AND tenant = ?2"),
        params![id.to_string(), tenant.as_str()],
        workflow_from,
    )
    .optional()
    .map_err(storage)?
    .map(build_workflow)
    .transpose()
}

fn require_workflow(db: &Db, tenant: &TenantId, id: WorkflowId) -> Result<Workflow> {
    load_workflow(db, tenant, id)?.ok_or_else(|| Error::Missing(format!("workflow {id}")))
}

type RawVersion = (String, u32, String, Option<String>, i64, String);

const VERSION_COLUMNS: &str = "workflow_id, version, hash, note, created_at, doc";

fn version_from(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawVersion> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?))
}

/// What the `doc` column of a version holds: the spec and the confirmations given with it.
#[derive(serde::Serialize, serde::Deserialize)]
struct VersionDoc {
    spec: WorkflowSpec,
    #[serde(default)]
    confirmations: Vec<Confirmation>,
}

fn build_version(raw: RawVersion) -> Result<WorkflowVersion> {
    let (workflow, version, hash, note, created, doc) = raw;
    let doc: VersionDoc = parse(&doc)?;
    Ok(WorkflowVersion {
        workflow: workflow.parse().map_err(storage)?,
        version,
        hash,
        spec: doc.spec,
        note,
        confirmations: doc.confirmations,
        created_at: Timestamp(created),
    })
}

fn load_version(db: &Db, id: WorkflowId, version: u32) -> Result<Option<WorkflowVersion>> {
    db.query_row(
        &format!("SELECT {VERSION_COLUMNS} FROM workflow_versions WHERE workflow_id = ?1 AND version = ?2"),
        params![id.to_string(), version],
        version_from,
    )
    .optional()
    .map_err(storage)?
    .map(build_version)
    .transpose()
}

/// Publish the draft of `workflow` inside a transaction. `carry` are confirmations that stay valid when the step they name is
/// unchanged.
fn publish_in(
    tx: &Db,
    workflow: &Workflow,
    note: Option<String>,
    confirm: Option<Confirm>,
    carry: &[Confirmation],
    now: Timestamp,
) -> Result<(WorkflowVersion, bool)> {
    let spec = workflow.draft.clone();
    spec.validate()?;
    let live = match workflow.live_version {
        Some(v) => load_version(tx, workflow.id, v)?,
        None => None,
    };
    if let Some(live) = &live
        && live.hash == spec.hash()
    {
        return Ok((live.clone(), false));
    }
    let next = workflow.live_version.map_or(1, |v| v + 1);
    if next as usize > MAX_WORKFLOW_VERSIONS {
        return Err(invalid(format!("A workflow keeps {MAX_WORKFLOW_VERSIONS} versions at most.")));
    }
    let mut confirmations = Vec::new();
    let mut missing = Vec::new();
    for step in spec.unattended() {
        let hash = step.hash();
        if let Some(kept) = carry.iter().find(|c| c.step == step.id && c.hash == hash) {
            confirmations.push(kept.clone());
        } else if let Some(confirm) = &confirm {
            confirmations.push(Confirmation { step: step.id.clone(), hash, by: confirm.by.clone(), at: now });
        } else {
            missing.push(step.id.clone());
        }
    }
    if !missing.is_empty() {
        return Err(invalid(format!(
            "A person must confirm that these steps run without asking first: {}.",
            missing.join(", ")
        )));
    }
    let doc = encode(&VersionDoc { spec: spec.clone(), confirmations: confirmations.clone() })?;
    tx.execute(
        "INSERT INTO workflow_versions (workflow_id, version, hash, note, created_at, doc) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![workflow.id.to_string(), next, spec.hash(), note, now.millis(), doc],
    )
    .map_err(storage)?;
    tx.execute(
        "UPDATE workflows SET live_version = ?2, updated_at = ?3 WHERE id = ?1",
        params![workflow.id.to_string(), next, now.millis()],
    )
    .map_err(storage)?;
    let version = WorkflowVersion {
        workflow: workflow.id,
        version: next,
        hash: spec.hash(),
        spec,
        note,
        confirmations,
        created_at: now,
    };
    Ok((version, true))
}

fn run_from(row: &rusqlite::Row<'_>) -> rusqlite::Result<(String, i64)> {
    Ok((row.get(0)?, row.get(1)?))
}

fn build_run((doc, revision): (String, i64)) -> Result<WorkflowRun> {
    let mut run: WorkflowRun = parse(&doc)?;
    run.revision = u64::try_from(revision).unwrap_or(0);
    Ok(run)
}

fn wait_from(row: &rusqlite::Row<'_>) -> rusqlite::Result<String> {
    row.get(0)
}

#[async_trait]
impl WorkflowStore for SqliteWorkflowStore {
    async fn workflows(&self, tenant: &TenantId) -> Result<Vec<Workflow>> {
        self.with(|db| {
            let mut statement = db
                .prepare(&format!("SELECT {WORKFLOW_COLUMNS} FROM workflows WHERE tenant = ?1 ORDER BY slug"))
                .map_err(storage)?;
            let rows = statement.query_map(params![tenant.as_str()], workflow_from).map_err(storage)?;
            rows.map(|r| build_workflow(r.map_err(storage)?)).collect()
        })
    }

    async fn workflow(&self, tenant: &TenantId, id: WorkflowId) -> Result<Option<Workflow>> {
        self.with(|db| load_workflow(db, tenant, id))
    }

    async fn workflow_by_slug(&self, tenant: &TenantId, slug: &WorkflowSlug) -> Result<Option<Workflow>> {
        self.with(|db| {
            db.query_row(
                &format!("SELECT {WORKFLOW_COLUMNS} FROM workflows WHERE tenant = ?1 AND slug = ?2"),
                params![tenant.as_str(), slug.as_str()],
                workflow_from,
            )
            .optional()
            .map_err(storage)?
            .map(build_workflow)
            .transpose()
        })
    }

    async fn create_workflow(
        &self,
        tenant: &TenantId,
        slug: WorkflowSlug,
        draft: WorkflowSpec,
        now: Timestamp,
    ) -> Result<Workflow> {
        draft.validate()?;
        self.with(|db| {
            let tx = db.transaction().map_err(storage)?;
            let count: i64 = tx
                .query_row("SELECT COUNT(*) FROM workflows WHERE tenant = ?1", params![tenant.as_str()], |r| r.get(0))
                .map_err(storage)?;
            if count as usize >= MAX_WORKFLOWS {
                return Err(invalid(format!("You can keep {MAX_WORKFLOWS} workflows at most.")));
            }
            let taken: Option<i64> = tx
                .query_row(
                    "SELECT 1 FROM workflows WHERE tenant = ?1 AND slug = ?2",
                    params![tenant.as_str(), slug.as_str()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(storage)?;
            if taken.is_some() {
                return Err(invalid(format!("A workflow called “{slug}” already exists.")));
            }
            let id = WorkflowId::new();
            tx.execute(
                "INSERT INTO workflows (id, tenant, slug, enabled, live_version, created_at, updated_at, doc)
                 VALUES (?1, ?2, ?3, 0, NULL, ?4, ?4, ?5)",
                params![id.to_string(), tenant.as_str(), slug.as_str(), now.millis(), encode(&draft)?],
            )
            .map_err(storage)?;
            tx.commit().map_err(storage)?;
            Ok(Workflow {
                id,
                tenant: tenant.clone(),
                slug,
                draft,
                live_version: None,
                enabled: false,
                created_at: now,
                updated_at: now,
            })
        })
    }

    async fn save_draft(
        &self,
        tenant: &TenantId,
        id: WorkflowId,
        draft: WorkflowSpec,
        now: Timestamp,
    ) -> Result<Workflow> {
        draft.validate()?;
        self.with(|db| {
            let tx = db.transaction().map_err(storage)?;
            let mut workflow = require_workflow(&tx, tenant, id)?;
            tx.execute(
                "UPDATE workflows SET doc = ?2, updated_at = ?3 WHERE id = ?1",
                params![id.to_string(), encode(&draft)?, now.millis()],
            )
            .map_err(storage)?;
            tx.commit().map_err(storage)?;
            workflow.draft = draft;
            workflow.updated_at = now;
            Ok(workflow)
        })
    }

    async fn publish(
        &self,
        tenant: &TenantId,
        id: WorkflowId,
        note: Option<String>,
        confirm: Option<Confirm>,
        now: Timestamp,
    ) -> Result<(WorkflowVersion, bool)> {
        self.with(|db| {
            let tx = db.transaction().map_err(storage)?;
            let workflow = require_workflow(&tx, tenant, id)?;
            let carry = match workflow.live_version {
                Some(v) => load_version(&tx, id, v)?.map(|v| v.confirmations).unwrap_or_default(),
                None => Vec::new(),
            };
            let published = publish_in(&tx, &workflow, note, confirm, &carry, now)?;
            tx.commit().map_err(storage)?;
            Ok(published)
        })
    }

    async fn versions(&self, tenant: &TenantId, id: WorkflowId) -> Result<Vec<WorkflowVersion>> {
        self.with(|db| {
            require_workflow(db, tenant, id)?;
            let mut statement = db
                .prepare(&format!(
                    "SELECT {VERSION_COLUMNS} FROM workflow_versions WHERE workflow_id = ?1 ORDER BY version DESC"
                ))
                .map_err(storage)?;
            let rows = statement.query_map(params![id.to_string()], version_from).map_err(storage)?;
            rows.map(|r| build_version(r.map_err(storage)?)).collect()
        })
    }

    async fn version(&self, tenant: &TenantId, id: WorkflowId, version: u32) -> Result<Option<WorkflowVersion>> {
        self.with(|db| {
            if load_workflow(db, tenant, id)?.is_none() {
                return Ok(None);
            }
            load_version(db, id, version)
        })
    }

    async fn rollback(
        &self,
        tenant: &TenantId,
        id: WorkflowId,
        to: u32,
        confirm: Option<Confirm>,
        now: Timestamp,
    ) -> Result<WorkflowVersion> {
        self.with(|db| {
            let tx = db.transaction().map_err(storage)?;
            let mut workflow = require_workflow(&tx, tenant, id)?;
            let old =
                load_version(&tx, id, to)?.ok_or_else(|| Error::Missing(format!("version {to} of workflow {id}")))?;
            let mut carry = old.confirmations.clone();
            if let Some(v) = workflow.live_version
                && let Some(live) = load_version(&tx, id, v)?
            {
                carry.extend(live.confirmations);
            }
            tx.execute(
                "UPDATE workflows SET doc = ?2, updated_at = ?3 WHERE id = ?1",
                params![id.to_string(), encode(&old.spec)?, now.millis()],
            )
            .map_err(storage)?;
            workflow.draft = old.spec;
            let (version, _) = publish_in(&tx, &workflow, Some(format!("rollback to v{to}")), confirm, &carry, now)?;
            tx.commit().map_err(storage)?;
            Ok(version)
        })
    }

    async fn set_enabled(&self, tenant: &TenantId, id: WorkflowId, enabled: bool, now: Timestamp) -> Result<()> {
        self.with(|db| {
            let workflow = require_workflow(db, tenant, id)?;
            if enabled && workflow.live_version.is_none() {
                return Err(invalid("Publish the workflow before turning it on: only a published version can run."));
            }
            db.execute(
                "UPDATE workflows SET enabled = ?2, updated_at = ?3 WHERE id = ?1",
                params![id.to_string(), enabled, now.millis()],
            )
            .map_err(storage)?;
            Ok(())
        })
    }

    async fn remove_workflow(&self, tenant: &TenantId, id: WorkflowId) -> Result<bool> {
        self.with(|db| {
            let tx = db.transaction().map_err(storage)?;
            if load_workflow(&tx, tenant, id)?.is_none() {
                return Ok(false);
            }
            for sql in [
                "DELETE FROM workflow_versions WHERE workflow_id = ?1",
                "DELETE FROM workflow_triggers WHERE workflow_id = ?1",
                "DELETE FROM workflows WHERE id = ?1",
            ] {
                tx.execute(sql, params![id.to_string()]).map_err(storage)?;
            }
            tx.commit().map_err(storage)?;
            Ok(true)
        })
    }

    async fn put_run(&self, tenant: &TenantId, run: &WorkflowRun, expected: Option<u64>) -> Result<bool> {
        self.with(|db| {
            let state = run.state.as_str();
            match expected {
                None => {
                    let mut stored = run.clone();
                    stored.revision = 0;
                    let changed = db
                        .execute(
                            "INSERT INTO workflow_runs (id, tenant, workflow_id, state, started_at, revision, doc)
                             VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6) ON CONFLICT (id) DO NOTHING",
                            params![
                                run.id.to_string(),
                                tenant.as_str(),
                                run.workflow.to_string(),
                                state,
                                run.started_at.millis(),
                                encode(&stored)?
                            ],
                        )
                        .map_err(storage)?;
                    Ok(changed == 1)
                }
                Some(revision) => {
                    let mut stored = run.clone();
                    stored.revision = revision + 1;
                    let (old, new) = (revision as i64, (revision + 1) as i64);
                    let changed = db
                        .execute(
                            "UPDATE workflow_runs SET state = ?3, revision = ?4, doc = ?5
                             WHERE id = ?1 AND tenant = ?2 AND revision = ?6",
                            params![run.id.to_string(), tenant.as_str(), state, new, encode(&stored)?, old],
                        )
                        .map_err(storage)?;
                    Ok(changed == 1)
                }
            }
        })
    }

    async fn run(&self, tenant: &TenantId, id: WorkflowRunId) -> Result<Option<WorkflowRun>> {
        self.with(|db| {
            db.query_row(
                "SELECT doc, revision FROM workflow_runs WHERE id = ?1 AND tenant = ?2",
                params![id.to_string(), tenant.as_str()],
                run_from,
            )
            .optional()
            .map_err(storage)?
            .map(build_run)
            .transpose()
        })
    }

    async fn runs(&self, tenant: &TenantId, workflow: Option<WorkflowId>, limit: usize) -> Result<Vec<WorkflowRun>> {
        self.with(|db| {
            let limit = i64::try_from(limit.clamp(1, 1000)).unwrap_or(1000);
            let mut found = Vec::new();
            match workflow {
                Some(id) => {
                    let mut statement = db
                        .prepare(
                            "SELECT doc, revision FROM workflow_runs WHERE tenant = ?1 AND workflow_id = ?2
                             ORDER BY started_at DESC, id DESC LIMIT ?3",
                        )
                        .map_err(storage)?;
                    let rows = statement
                        .query_map(params![tenant.as_str(), id.to_string(), limit], run_from)
                        .map_err(storage)?;
                    for row in rows {
                        found.push(build_run(row.map_err(storage)?)?);
                    }
                }
                None => {
                    let mut statement = db
                        .prepare(
                            "SELECT doc, revision FROM workflow_runs WHERE tenant = ?1
                             ORDER BY started_at DESC, id DESC LIMIT ?2",
                        )
                        .map_err(storage)?;
                    let rows = statement.query_map(params![tenant.as_str(), limit], run_from).map_err(storage)?;
                    for row in rows {
                        found.push(build_run(row.map_err(storage)?)?);
                    }
                }
            }
            Ok(found)
        })
    }

    async fn active_runs(&self, tenant: &TenantId) -> Result<Vec<WorkflowRun>> {
        self.with(|db| {
            let mut statement = db
                .prepare(
                    "SELECT doc, revision FROM workflow_runs WHERE tenant = ?1 AND state IN ('running', 'waiting')
                     ORDER BY started_at",
                )
                .map_err(storage)?;
            let rows = statement.query_map(params![tenant.as_str()], run_from).map_err(storage)?;
            rows.map(|r| build_run(r.map_err(storage)?)).collect()
        })
    }

    async fn put_wait(&self, tenant: &TenantId, wait: &WorkflowWait) -> Result<()> {
        self.with(|db| {
            db.execute(
                "INSERT INTO workflow_waits (id, tenant, run_id, status, due_at, created_at, doc)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    wait.id.to_string(),
                    tenant.as_str(),
                    wait.run.to_string(),
                    wait.status.as_str(),
                    wait.due_at.map(|t| t.millis()),
                    wait.created_at.millis(),
                    encode(wait)?
                ],
            )
            .map_err(storage)?;
            Ok(())
        })
    }

    async fn wait(&self, tenant: &TenantId, id: WaitId) -> Result<Option<WorkflowWait>> {
        self.with(|db| {
            db.query_row(
                "SELECT doc FROM workflow_waits WHERE id = ?1 AND tenant = ?2",
                params![id.to_string(), tenant.as_str()],
                wait_from,
            )
            .optional()
            .map_err(storage)?
            .map(|doc| parse::<WorkflowWait>(&doc))
            .transpose()
        })
    }

    async fn waits_of(&self, tenant: &TenantId, run: WorkflowRunId) -> Result<Vec<WorkflowWait>> {
        self.with(|db| {
            let mut statement = db
                .prepare("SELECT doc FROM workflow_waits WHERE tenant = ?1 AND run_id = ?2 ORDER BY created_at, id")
                .map_err(storage)?;
            let rows = statement.query_map(params![tenant.as_str(), run.to_string()], wait_from).map_err(storage)?;
            rows.map(|r| parse::<WorkflowWait>(&r.map_err(storage)?)).collect()
        })
    }

    async fn pending_waits(&self, tenant: &TenantId) -> Result<Vec<WorkflowWait>> {
        self.with(|db| {
            let mut statement = db
                .prepare(
                    "SELECT doc FROM workflow_waits WHERE tenant = ?1 AND status = 'pending' ORDER BY created_at, id",
                )
                .map_err(storage)?;
            let rows = statement.query_map(params![tenant.as_str()], wait_from).map_err(storage)?;
            rows.map(|r| parse::<WorkflowWait>(&r.map_err(storage)?)).collect()
        })
    }

    async fn settle_wait(
        &self,
        tenant: &TenantId,
        id: WaitId,
        decision: WaitDecision,
        now: Timestamp,
    ) -> Result<Option<WorkflowWait>> {
        self.with(|db| {
            let tx = db.transaction().map_err(storage)?;
            let doc: Option<String> = tx
                .query_row(
                    "SELECT doc FROM workflow_waits WHERE id = ?1 AND tenant = ?2 AND status = 'pending'",
                    params![id.to_string(), tenant.as_str()],
                    wait_from,
                )
                .optional()
                .map_err(storage)?;
            let Some(doc) = doc else { return Ok(None) };
            let mut wait: WorkflowWait = parse(&doc)?;
            let status = decision.status;
            wait.status = status;
            wait.decided_at = Some(now);
            wait.decided_by = decision.by;
            wait.answer = decision.answer;
            wait.note = decision.note;
            tx.execute(
                "UPDATE workflow_waits SET status = ?2, doc = ?3 WHERE id = ?1",
                params![id.to_string(), status.as_str(), encode(&wait)?],
            )
            .map_err(storage)?;
            tx.commit().map_err(storage)?;
            Ok(Some(wait))
        })
    }

    async fn mark_reminded(&self, tenant: &TenantId, id: WaitId) -> Result<()> {
        self.with(|db| {
            let doc: Option<String> = db
                .query_row(
                    "SELECT doc FROM workflow_waits WHERE id = ?1 AND tenant = ?2",
                    params![id.to_string(), tenant.as_str()],
                    wait_from,
                )
                .optional()
                .map_err(storage)?;
            if let Some(doc) = doc {
                let mut wait: WorkflowWait = parse(&doc)?;
                wait.reminded = true;
                db.execute("UPDATE workflow_waits SET doc = ?2 WHERE id = ?1", params![id.to_string(), encode(&wait)?])
                    .map_err(storage)?;
            }
            Ok(())
        })
    }

    async fn trigger_states(&self, tenant: &TenantId) -> Result<Vec<TriggerState>> {
        self.with(|db| {
            let mut statement = db.prepare("SELECT doc FROM workflow_triggers WHERE tenant = ?1").map_err(storage)?;
            let rows = statement.query_map(params![tenant.as_str()], wait_from).map_err(storage)?;
            rows.map(|r| parse::<TriggerState>(&r.map_err(storage)?)).collect()
        })
    }

    async fn put_trigger_state(&self, tenant: &TenantId, state: &TriggerState) -> Result<()> {
        self.with(|db| {
            db.execute(
                "INSERT INTO workflow_triggers (tenant, workflow_id, step, doc) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (tenant, workflow_id, step) DO UPDATE SET doc = excluded.doc",
                params![tenant.as_str(), state.workflow.to_string(), state.step, encode(state)?],
            )
            .map_err(storage)?;
            Ok(())
        })
    }

    async fn clear_trigger_states(&self, tenant: &TenantId, workflow: WorkflowId) -> Result<()> {
        self.with(|db| {
            db.execute(
                "DELETE FROM workflow_triggers WHERE tenant = ?1 AND workflow_id = ?2",
                params![tenant.as_str(), workflow.to_string()],
            )
            .map_err(storage)?;
            Ok(())
        })
    }

    async fn meta(&self, tenant: &TenantId, key: &str) -> Result<Option<String>> {
        self.with(|db| {
            db.query_row(
                "SELECT value FROM workflow_meta WHERE tenant = ?1 AND key = ?2",
                params![tenant.as_str(), key],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage)
        })
    }

    async fn record_delivery(
        &self,
        tenant: &TenantId,
        workflow: WorkflowId,
        key: &str,
        now: Timestamp,
        keep_ms: i64,
    ) -> Result<bool> {
        self.with(|db| {
            let (tenant, workflow) = (tenant.as_str(), workflow.to_string());
            let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(storage)?;
            tx.execute(
                "DELETE FROM workflow_deliveries WHERE tenant = ?1 AND workflow_id = ?2 AND at < ?3",
                params![tenant, workflow, now.millis() - keep_ms],
            )
            .map_err(storage)?;
            let first = tx
                .execute(
                    "INSERT OR IGNORE INTO workflow_deliveries (tenant, workflow_id, key, at) VALUES (?1, ?2, ?3, ?4)",
                    params![tenant, workflow, key, now.millis()],
                )
                .map_err(storage)?
                == 1;
            // A flood of distinct ids cannot grow the table without bound: only the newest ones are kept.
            tx.execute(
                "DELETE FROM workflow_deliveries WHERE tenant = ?1 AND workflow_id = ?2 AND key IN (
                     SELECT key FROM workflow_deliveries WHERE tenant = ?1 AND workflow_id = ?2
                     ORDER BY at DESC, key LIMIT -1 OFFSET 10000)",
                params![tenant, workflow],
            )
            .map_err(storage)?;
            tx.commit().map_err(storage)?;
            Ok(first)
        })
    }

    async fn set_meta(&self, tenant: &TenantId, key: &str, value: &str) -> Result<()> {
        self.with(|db| {
            db.execute(
                "INSERT INTO workflow_meta (tenant, key, value) VALUES (?1, ?2, ?3)
                 ON CONFLICT (tenant, key) DO UPDATE SET value = excluded.value",
                params![tenant.as_str(), key, value],
            )
            .map_err(storage)?;
            Ok(())
        })
    }
}
