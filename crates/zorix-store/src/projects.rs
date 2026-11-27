//! The SQLite implementation of [`ProjectStore`] (ADR-0036). It opens its own connection like the other stores and every query
//! carries the tenant. A task is written with its events in one transaction, and a task number is never given twice.

use std::path::Path;
use std::sync::Arc;

use crate::errors::{encode_redacted, storage};
use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::{Connection as Db, OptionalExtension, Transaction, TransactionBehavior, params};
use zorix_core::project::MAX_PROJECT_TASKS;
use zorix_core::{
    Error, MAX_PROJECTS, Project, ProjectEvent, ProjectEventDraft, ProjectEventKind, ProjectId, ProjectKey,
    ProjectStore, Result, Task, TaskCount, TaskKey, TaskState, TenantId, Who,
};

/// Stores projects in the same SQLite file as everything else.
#[derive(Clone)]
pub struct SqliteProjectStore {
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

fn state_text(state: TaskState) -> &'static str {
    state.as_str()
}

impl SqliteProjectStore {
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

type RawProject = (u32, String);

fn project_from(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawProject> {
    Ok((row.get(0)?, row.get(1)?))
}

const PROJECT_COLUMNS: &str = "next_task, doc";

fn build_project(raw: RawProject) -> Result<Project> {
    let (next, doc) = raw;
    let mut project: Project = parse(&doc)?;
    project.next_task = next;
    Ok(project)
}

fn load_project(db: &Db, tenant: &TenantId, id: ProjectId) -> Result<Option<Project>> {
    db.query_row(
        &format!("SELECT {PROJECT_COLUMNS} FROM projects WHERE id = ?1 AND tenant = ?2"),
        params![id.to_string(), tenant.as_str()],
        project_from,
    )
    .optional()
    .map_err(storage)?
    .map(build_project)
    .transpose()
}

type RawTask = (u64, String);

fn task_from(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawTask> {
    let revision: i64 = row.get(0)?;
    Ok((u64::try_from(revision).unwrap_or(0), row.get(1)?))
}

const TASK_COLUMNS: &str = "revision, doc";

fn build_task(raw: RawTask) -> Result<Task> {
    let (revision, doc) = raw;
    let mut task: Task = parse(&doc)?;
    task.revision = revision;
    Ok(task)
}

fn load_task(db: &Db, tenant: &TenantId, key: &TaskKey) -> Result<Option<Task>> {
    db.query_row(
        &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE tenant = ?1 AND key = ?2"),
        params![tenant.as_str(), key.to_string()],
        task_from,
    )
    .optional()
    .map_err(storage)?
    .map(build_task)
    .transpose()
}

fn tasks_of(db: &Db, tenant: &TenantId, project: ProjectId) -> Result<Vec<Task>> {
    let mut statement = db
        .prepare(&format!("SELECT {TASK_COLUMNS} FROM tasks WHERE tenant = ?1 AND project_id = ?2 ORDER BY rank, id"))
        .map_err(storage)?;
    let rows = statement
        .query_map(params![tenant.as_str(), project.to_string()], task_from)
        .map_err(storage)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(storage)?;
    rows.into_iter().map(build_task).collect()
}

/// Number and store events of a project, after the highest number it has.
fn insert_events(
    tx: &Transaction<'_>,
    tenant: &TenantId,
    project: ProjectId,
    drafts: &[ProjectEventDraft],
) -> Result<()> {
    if drafts.is_empty() {
        return Ok(());
    }
    let mut seq: u64 = tx
        .query_row(
            "SELECT COALESCE(MAX(seq), 0) FROM project_events WHERE project_id = ?1",
            [project.to_string()],
            |r| r.get::<_, i64>(0),
        )
        .map_err(storage)
        .map(|n| u64::try_from(n).unwrap_or(0))?;
    for draft in drafts {
        seq += 1;
        let event = ProjectEvent {
            seq,
            project,
            task: draft.task.clone(),
            by: draft.by.clone(),
            kind: draft.kind.clone(),
            at: draft.at,
        };
        tx.execute(
            "INSERT INTO project_events (tenant, project_id, seq, task, at, doc) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                tenant.as_str(),
                project.to_string(),
                i64::try_from(seq).map_err(storage)?,
                draft.task.as_ref().map(ToString::to_string),
                draft.at.millis(),
                encode_redacted(&event)?
            ],
        )
        .map_err(storage)?;
    }
    Ok(())
}

#[async_trait]
impl ProjectStore for SqliteProjectStore {
    async fn projects(&self, tenant: &TenantId) -> Result<Vec<Project>> {
        self.with(|db| {
            let mut statement = db
                .prepare(&format!("SELECT {PROJECT_COLUMNS} FROM projects WHERE tenant = ?1 ORDER BY key"))
                .map_err(storage)?;
            let rows = statement
                .query_map([tenant.as_str()], project_from)
                .map_err(storage)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(storage)?;
            rows.into_iter().map(build_project).collect()
        })
    }

    async fn project(&self, tenant: &TenantId, id: ProjectId) -> Result<Option<Project>> {
        self.with(|db| load_project(db, tenant, id))
    }

    async fn project_by_key(&self, tenant: &TenantId, key: &ProjectKey) -> Result<Option<Project>> {
        self.with(|db| {
            db.query_row(
                &format!("SELECT {PROJECT_COLUMNS} FROM projects WHERE tenant = ?1 AND key = ?2"),
                params![tenant.as_str(), key.as_str()],
                project_from,
            )
            .optional()
            .map_err(storage)?
            .map(build_project)
            .transpose()
        })
    }

    async fn create_project(&self, tenant: &TenantId, mut project: Project, by: Option<Who>) -> Result<Project> {
        project.validate()?;
        project.tenant = tenant.clone();
        project.next_task = 1;
        self.with(|db| {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
            let count: i64 = tx
                .query_row("SELECT COUNT(*) FROM projects WHERE tenant = ?1", [tenant.as_str()], |r| r.get(0))
                .map_err(storage)?;
            if count >= MAX_PROJECTS as i64 {
                return Err(invalid(format!("A tenant keeps at most {MAX_PROJECTS} projects. Delete one first.")));
            }
            let taken: bool = tx
                .query_row(
                    "SELECT 1 FROM projects WHERE tenant = ?1 AND key = ?2",
                    params![tenant.as_str(), project.key.as_str()],
                    |_| Ok(true),
                )
                .optional()
                .map_err(storage)?
                .unwrap_or(false);
            if taken {
                return Err(invalid(format!("The key {} is already used by another project.", project.key)));
            }
            tx.execute(
                "INSERT INTO projects (id, tenant, key, archived, next_task, created_at, updated_at, doc) VALUES (?1, ?2, ?3, 0, 1, ?4, ?5, ?6)",
                params![
                    project.id.to_string(),
                    tenant.as_str(),
                    project.key.as_str(),
                    project.created_at.millis(),
                    project.updated_at.millis(),
                    encode(&project)?
                ],
            )
            .map_err(storage)?;
            insert_events(
                &tx,
                tenant,
                project.id,
                &[ProjectEventDraft { task: None, by, kind: ProjectEventKind::ProjectCreated, at: project.created_at }],
            )?;
            tx.commit().map_err(storage)?;
            Ok(project)
        })
    }

    async fn update_project(
        &self,
        tenant: &TenantId,
        project: &Project,
        events: &[ProjectEventDraft],
    ) -> Result<Project> {
        project.validate()?;
        self.with(|db| {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
            let stored = load_project(&tx, tenant, project.id)?
                .ok_or_else(|| Error::Missing(format!("project {}", project.key)))?;
            let mut next = project.clone();
            next.tenant = stored.tenant.clone();
            next.key = stored.key.clone();
            next.next_task = stored.next_task;
            next.created_at = stored.created_at;
            tx.execute(
                "UPDATE projects SET archived = ?1, updated_at = ?2, doc = ?3 WHERE id = ?4 AND tenant = ?5",
                params![next.archived, next.updated_at.millis(), encode(&next)?, next.id.to_string(), tenant.as_str()],
            )
            .map_err(storage)?;
            insert_events(&tx, tenant, next.id, events)?;
            tx.commit().map_err(storage)?;
            Ok(next)
        })
    }

    async fn remove_project(&self, tenant: &TenantId, id: ProjectId) -> Result<bool> {
        self.with(|db| {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
            let removed = tx
                .execute("DELETE FROM projects WHERE id = ?1 AND tenant = ?2", params![id.to_string(), tenant.as_str()])
                .map_err(storage)?;
            if removed > 0 {
                tx.execute(
                    "DELETE FROM tasks WHERE project_id = ?1 AND tenant = ?2",
                    params![id.to_string(), tenant.as_str()],
                )
                .map_err(storage)?;
                tx.execute(
                    "DELETE FROM project_events WHERE project_id = ?1 AND tenant = ?2",
                    params![id.to_string(), tenant.as_str()],
                )
                .map_err(storage)?;
            }
            tx.commit().map_err(storage)?;
            Ok(removed > 0)
        })
    }

    async fn next_task_number(&self, tenant: &TenantId, project: ProjectId) -> Result<u32> {
        self.with(|db| {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
            let current: u32 = tx
                .query_row(
                    "SELECT next_task FROM projects WHERE id = ?1 AND tenant = ?2",
                    params![project.to_string(), tenant.as_str()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(storage)?
                .ok_or_else(|| Error::Missing(format!("project {project}")))?;
            tx.execute(
                "UPDATE projects SET next_task = ?1 WHERE id = ?2 AND tenant = ?3",
                params![current + 1, project.to_string(), tenant.as_str()],
            )
            .map_err(storage)?;
            tx.commit().map_err(storage)?;
            Ok(current)
        })
    }

    async fn put_task(
        &self,
        tenant: &TenantId,
        task: &Task,
        expected: Option<u64>,
        events: &[ProjectEventDraft],
    ) -> Result<bool> {
        task.validate()?;
        self.with(|db| {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
            if load_project(&tx, tenant, task.project)?.is_none() {
                return Err(Error::Missing(format!("project of {}", task.key)));
            }
            let doc = encode(task)?;
            match expected {
                None => {
                    let count: i64 = tx
                        .query_row(
                            "SELECT COUNT(*) FROM tasks WHERE tenant = ?1 AND project_id = ?2",
                            params![tenant.as_str(), task.project.to_string()],
                            |r| r.get(0),
                        )
                        .map_err(storage)?;
                    if count >= MAX_PROJECT_TASKS as i64 {
                        return Err(invalid(format!("A project keeps at most {MAX_PROJECT_TASKS} tasks.")));
                    }
                    let inserted = tx
                        .execute(
                            "INSERT INTO tasks (id, tenant, project_id, key, state, rank, revision, updated_at, doc) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8) ON CONFLICT DO NOTHING",
                            params![
                                task.id.to_string(),
                                tenant.as_str(),
                                task.project.to_string(),
                                task.key.to_string(),
                                state_text(task.state),
                                task.rank,
                                task.updated_at.millis(),
                                doc
                            ],
                        )
                        .map_err(storage)?;
                    if inserted == 0 {
                        return Ok(false);
                    }
                }
                Some(revision) => {
                    let updated = tx
                        .execute(
                            "UPDATE tasks SET state = ?1, rank = ?2, revision = revision + 1, updated_at = ?3, doc = ?4 WHERE id = ?5 AND tenant = ?6 AND revision = ?7",
                            params![
                                state_text(task.state),
                                task.rank,
                                task.updated_at.millis(),
                                doc,
                                task.id.to_string(),
                                tenant.as_str(),
                                i64::try_from(revision).map_err(storage)?
                            ],
                        )
                        .map_err(storage)?;
                    if updated == 0 {
                        return Ok(false);
                    }
                }
            }
            insert_events(&tx, tenant, task.project, events)?;
            tx.commit().map_err(storage)?;
            Ok(true)
        })
    }

    async fn task(&self, tenant: &TenantId, key: &TaskKey) -> Result<Option<Task>> {
        self.with(|db| load_task(db, tenant, key))
    }

    async fn tasks(&self, tenant: &TenantId, project: ProjectId) -> Result<Vec<Task>> {
        self.with(|db| tasks_of(db, tenant, project))
    }

    async fn tasks_in_state(&self, tenant: &TenantId, state: TaskState) -> Result<Vec<Task>> {
        self.with(|db| {
            let mut statement = db
                .prepare(
                    "SELECT t.revision, t.doc FROM tasks t JOIN projects p ON p.id = t.project_id WHERE t.tenant = ?1 AND t.state = ?2 AND p.archived = 0 ORDER BY t.updated_at, t.id",
                )
                .map_err(storage)?;
            let rows = statement
                .query_map(params![tenant.as_str(), state_text(state)], task_from)
                .map_err(storage)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(storage)?;
            rows.into_iter().map(build_task).collect()
        })
    }

    async fn remove_task(&self, tenant: &TenantId, key: &TaskKey) -> Result<bool> {
        self.with(|db| {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
            let Some(task) = load_task(&tx, tenant, key)? else {
                return Ok(false);
            };
            tx.execute(
                "DELETE FROM tasks WHERE id = ?1 AND tenant = ?2",
                params![task.id.to_string(), tenant.as_str()],
            )
            .map_err(storage)?;
            for mut other in tasks_of(&tx, tenant, task.project)? {
                if other.depends_on.contains(key) {
                    other.depends_on.retain(|k| k != key);
                    tx.execute(
                        "UPDATE tasks SET revision = revision + 1, doc = ?1 WHERE id = ?2 AND tenant = ?3",
                        params![encode(&other)?, other.id.to_string(), tenant.as_str()],
                    )
                    .map_err(storage)?;
                }
            }
            tx.commit().map_err(storage)?;
            Ok(true)
        })
    }

    async fn task_counts(&self, tenant: &TenantId) -> Result<Vec<TaskCount>> {
        self.with(|db| {
            let mut statement = db
                .prepare("SELECT project_id, state, COUNT(*) FROM tasks WHERE tenant = ?1 GROUP BY project_id, state")
                .map_err(storage)?;
            let rows = statement
                .query_map([tenant.as_str()], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, u32>(2)?))
                })
                .map_err(storage)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(storage)?;
            rows.into_iter()
                .map(|(project, state, count)| {
                    Ok(TaskCount {
                        project: project.parse().map_err(storage)?,
                        state: state.parse().map_err(storage)?,
                        count,
                    })
                })
                .collect()
        })
    }

    async fn add_events(&self, tenant: &TenantId, project: ProjectId, events: &[ProjectEventDraft]) -> Result<()> {
        self.with(|db| {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
            if load_project(&tx, tenant, project)?.is_none() {
                return Err(Error::Missing(format!("project {project}")));
            }
            insert_events(&tx, tenant, project, events)?;
            tx.commit().map_err(storage)
        })
    }

    async fn events(
        &self,
        tenant: &TenantId,
        project: ProjectId,
        task: Option<&TaskKey>,
        limit: usize,
    ) -> Result<Vec<ProjectEvent>> {
        self.with(|db| {
            let limit = i64::try_from(limit.clamp(1, 1000)).map_err(storage)?;
            let mut statement = db
                .prepare(
                    "SELECT doc FROM project_events WHERE tenant = ?1 AND project_id = ?2 AND (?3 IS NULL OR task = ?3) ORDER BY seq DESC LIMIT ?4",
                )
                .map_err(storage)?;
            let rows = statement
                .query_map(
                    params![tenant.as_str(), project.to_string(), task.map(ToString::to_string), limit],
                    |r| r.get::<_, String>(0),
                )
                .map_err(storage)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(storage)?;
            rows.iter().map(|doc| parse(doc)).collect()
        })
    }
}
