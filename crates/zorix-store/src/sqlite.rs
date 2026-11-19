//! The SQLite implementation of [`ExecutionRepository`] (ADR-0004).

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params, params_from_iter};
use serde_json::Value;
use zorix_core::{
    Approval, ApprovalId, Checkpoint, Commit, Committed, CreateOutcome, EVENT_SCHEMA_VERSION, Error, Execution,
    ExecutionEvent, ExecutionId, ExecutionRepository, ExecutionState, ListFilter, NewEvent, Result, TenantId,
    Timestamp, ToolCallRecord,
};

use crate::schema;

/// Stores everything in one SQLite file (or in memory, for tests).
#[derive(Clone)]
pub struct SqliteRepository {
    conn: Arc<Mutex<Connection>>,
}

fn storage(error: impl std::fmt::Display) -> Error {
    Error::Storage(error.to_string())
}

fn parse<T: serde::de::DeserializeOwned>(text: &str) -> Result<T> {
    serde_json::from_str(text).map_err(|e| Error::Storage(format!("a stored record could not be read: {e}")))
}

fn encode<T: serde::Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(storage)
}

impl SqliteRepository {
    /// Open (or create) the database at `path`, and bring its schema up to date.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Storage(format!("could not create {}: {e}", parent.display())))?;
        }
        let conn = Connection::open(path).map_err(storage)?;
        Self::prepare(conn, true)
    }

    /// A database that lives in memory and disappears when dropped.
    pub fn open_in_memory() -> Result<Self> {
        Self::prepare(Connection::open_in_memory().map_err(storage)?, false)
    }

    fn prepare(mut conn: Connection, file: bool) -> Result<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(storage)?;
        if file {
            // WAL lets readers (the CLI looking around) work while the engine writes; NORMAL is safe with WAL.
            conn.pragma_update(None, "journal_mode", "WAL").map_err(storage)?;
            conn.pragma_update(None, "synchronous", "NORMAL").map_err(storage)?;
        }
        schema::migrate(&mut conn).map_err(Error::Storage)?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    /// Run blocking database work off the async threads.
    async fn run<T, F>(&self, work: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || work(&mut conn.lock()))
            .await
            .map_err(|e| Error::Internal(format!("a storage task failed: {e}")))?
    }
}

fn append_events(tx: &Transaction<'_>, id: ExecutionId, events: Vec<NewEvent>) -> Result<Vec<ExecutionEvent>> {
    let mut stored = Vec::with_capacity(events.len());
    let mut insert = tx
        .prepare_cached(
            "INSERT INTO events (execution_id, at, schema_version, kind, payload) VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .map_err(storage)?;
    for event in events {
        insert
            .execute(params![
                id.to_string(),
                event.at.millis(),
                EVENT_SCHEMA_VERSION,
                event.kind.name(),
                event.payload.to_string()
            ])
            .map_err(storage)?;
        stored.push(ExecutionEvent {
            seq: tx.last_insert_rowid() as u64,
            execution_id: id,
            at: event.at,
            schema_version: EVENT_SCHEMA_VERSION,
            kind: event.kind,
            payload: event.payload,
        });
    }
    Ok(stored)
}

fn write_records(
    tx: &Transaction<'_>,
    id: ExecutionId,
    checkpoint: Option<&Checkpoint>,
    tool_calls: &[ToolCallRecord],
    approvals: &[Approval],
) -> Result<()> {
    if let Some(checkpoint) = checkpoint {
        tx.execute(
            "INSERT INTO checkpoints (execution_id, step, created_at, doc) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(execution_id) DO UPDATE SET step = excluded.step, created_at = excluded.created_at, doc = excluded.doc",
            params![id.to_string(), checkpoint.step as i64, checkpoint.created_at.millis(), encode(checkpoint)?],
        )
        .map_err(storage)?;
    }
    for record in tool_calls {
        tx.execute(
            "INSERT INTO tool_calls (execution_id, call_id, ord, doc)
             VALUES (?1, ?2, (SELECT COALESCE(MAX(ord), 0) + 1 FROM tool_calls WHERE execution_id = ?1), ?3)
             ON CONFLICT(execution_id, call_id) DO UPDATE SET doc = excluded.doc",
            params![id.to_string(), record.call_id.0, encode(record)?],
        )
        .map_err(storage)?;
    }
    for approval in approvals {
        tx.execute(
            "INSERT INTO approvals (id, execution_id, ord, doc)
             VALUES (?1, ?2, (SELECT COALESCE(MAX(ord), 0) + 1 FROM approvals WHERE execution_id = ?2), ?3)
             ON CONFLICT(id) DO UPDATE SET doc = excluded.doc",
            params![approval.id.to_string(), id.to_string(), encode(approval)?],
        )
        .map_err(storage)?;
    }
    Ok(())
}

fn documents<T: serde::de::DeserializeOwned>(
    conn: &Connection,
    sql: &str,
    args: &[&dyn rusqlite::ToSql],
) -> Result<Vec<T>> {
    let mut statement = conn.prepare_cached(sql).map_err(storage)?;
    let rows = statement.query_map(params_from_iter(args.iter()), |row| row.get::<_, String>(0)).map_err(storage)?;
    rows.map(|row| parse(&row.map_err(storage)?)).collect()
}

#[async_trait]
impl ExecutionRepository for SqliteRepository {
    async fn create(&self, execution: Execution, events: Vec<NewEvent>) -> Result<CreateOutcome> {
        self.run(move |conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
            if let Some(key) = &execution.idempotency_key {
                let existing: Option<String> = tx
                    .query_row(
                        "SELECT doc FROM executions WHERE tenant = ?1 AND idempotency_key = ?2",
                        params![execution.tenant.as_str(), key],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(storage)?;
                if let Some(doc) = existing {
                    return Ok(CreateOutcome::Existing(parse(&doc)?));
                }
            }
            let mut stored = execution;
            stored.version = 1;
            tx.execute(
                "INSERT INTO executions (id, tenant, conversation, state, version, idempotency_key, created_at, doc, agent)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    stored.id.to_string(),
                    stored.tenant.as_str(),
                    stored.conversation.as_ref().map(|c| c.as_str().to_string()),
                    stored.state.as_str(),
                    stored.version as i64,
                    stored.idempotency_key,
                    stored.created_at.millis(),
                    encode(&stored)?,
                    stored.agent.as_ref().map(|a| a.agent.to_string()),
                ],
            )
            .map_err(storage)?;
            let events = append_events(&tx, stored.id, events)?;
            tx.commit().map_err(storage)?;
            Ok(CreateOutcome::Created(Committed { execution: stored, events }))
        })
        .await
    }

    async fn commit(&self, commit: Commit) -> Result<Committed> {
        self.run(move |conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
            let id = commit.execution.id;
            let current: Option<i64> = tx
                .query_row("SELECT version FROM executions WHERE id = ?1", params![id.to_string()], |row| row.get(0))
                .optional()
                .map_err(storage)?;
            match current {
                None => return Err(Error::NotFound(id)),
                Some(version) if version as u64 != commit.expected_version => return Err(Error::Conflict(id)),
                Some(_) => {}
            }
            let mut stored = commit.execution;
            stored.version = commit.expected_version + 1;
            tx.execute(
                "UPDATE executions SET state = ?2, version = ?3, doc = ?4 WHERE id = ?1",
                params![id.to_string(), stored.state.as_str(), stored.version as i64, encode(&stored)?],
            )
            .map_err(storage)?;
            let events = append_events(&tx, id, commit.events)?;
            write_records(&tx, id, commit.checkpoint.as_ref(), &commit.tool_calls, &commit.approvals)?;
            tx.commit().map_err(storage)?;
            Ok(Committed { execution: stored, events })
        })
        .await
    }

    async fn load(&self, id: ExecutionId) -> Result<Option<Execution>> {
        self.run(move |conn| {
            let doc: Option<String> = conn
                .query_row("SELECT doc FROM executions WHERE id = ?1", params![id.to_string()], |row| row.get(0))
                .optional()
                .map_err(storage)?;
            doc.map(|doc| parse(&doc)).transpose()
        })
        .await
    }

    async fn get(&self, tenant: &TenantId, id: ExecutionId) -> Result<Option<Execution>> {
        let tenant = tenant.clone();
        self.run(move |conn| {
            let doc: Option<String> = conn
                .query_row(
                    "SELECT doc FROM executions WHERE id = ?1 AND tenant = ?2",
                    params![id.to_string(), tenant.as_str()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(storage)?;
            doc.map(|doc| parse(&doc)).transpose()
        })
        .await
    }

    async fn list(&self, tenant: &TenantId, filter: &ListFilter) -> Result<Vec<Execution>> {
        let (tenant, filter) = (tenant.clone(), filter.clone());
        self.run(move |conn| {
            let mut sql = String::from("SELECT doc FROM executions WHERE tenant = ?");
            let mut args: Vec<String> = vec![tenant.as_str().to_string()];
            if !filter.states.is_empty() {
                sql.push_str(&format!(" AND state IN ({})", vec!["?"; filter.states.len()].join(",")));
                args.extend(filter.states.iter().map(|s| s.as_str().to_string()));
            }
            if let Some(conversation) = &filter.conversation {
                sql.push_str(" AND conversation = ?");
                args.push(conversation.as_str().to_string());
            }
            if let Some(agent) = &filter.agent {
                sql.push_str(" AND agent = ?");
                args.push(agent.to_string());
            }
            sql.push_str(" ORDER BY created_at DESC, id DESC LIMIT ?");
            let limit = if filter.limit == 0 { 100 } else { filter.limit };
            let mut statement = conn.prepare(&sql).map_err(storage)?;
            let mut bound: Vec<&dyn rusqlite::ToSql> = args.iter().map(|a| a as &dyn rusqlite::ToSql).collect();
            let limit = limit as i64;
            bound.push(&limit);
            let rows = statement.query_map(params_from_iter(bound), |row| row.get::<_, String>(0)).map_err(storage)?;
            rows.map(|row| parse(&row.map_err(storage)?)).collect()
        })
        .await
    }

    async fn in_states(&self, states: &[ExecutionState]) -> Result<Vec<Execution>> {
        let names: Vec<String> = states.iter().map(|s| s.as_str().to_string()).collect();
        if names.is_empty() {
            return Ok(Vec::new());
        }
        self.run(move |conn| {
            let sql = format!(
                "SELECT doc FROM executions WHERE state IN ({}) ORDER BY created_at, id",
                vec!["?"; names.len()].join(",")
            );
            let args: Vec<&dyn rusqlite::ToSql> = names.iter().map(|n| n as &dyn rusqlite::ToSql).collect();
            documents(conn, &sql, &args)
        })
        .await
    }

    async fn events(&self, id: ExecutionId, after_seq: u64, limit: usize) -> Result<Vec<ExecutionEvent>> {
        self.run(move |conn| {
            let mut statement = conn
                .prepare_cached(
                    "SELECT seq, at, schema_version, kind, payload FROM events
                     WHERE execution_id = ?1 AND seq > ?2 ORDER BY seq LIMIT ?3",
                )
                .map_err(storage)?;
            let rows = statement
                .query_map(params![id.to_string(), after_seq as i64, limit as i64], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, u32>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                })
                .map_err(storage)?;
            rows.map(|row| {
                let (seq, at, schema_version, kind, payload) = row.map_err(storage)?;
                Ok(ExecutionEvent {
                    seq: seq as u64,
                    execution_id: id,
                    at: Timestamp(at),
                    schema_version,
                    kind: serde_json::from_value(Value::String(kind.clone()))
                        .map_err(|_| Error::Storage(format!("an event has an unknown kind: {kind}")))?,
                    payload: parse(&payload)?,
                })
            })
            .collect()
        })
        .await
    }

    async fn checkpoint(&self, id: ExecutionId) -> Result<Option<Checkpoint>> {
        self.run(move |conn| {
            let doc: Option<String> = conn
                .query_row("SELECT doc FROM checkpoints WHERE execution_id = ?1", params![id.to_string()], |row| {
                    row.get(0)
                })
                .optional()
                .map_err(storage)?;
            doc.map(|doc| parse(&doc)).transpose()
        })
        .await
    }

    async fn tool_calls(&self, id: ExecutionId) -> Result<Vec<ToolCallRecord>> {
        self.run(move |conn| {
            documents(conn, "SELECT doc FROM tool_calls WHERE execution_id = ?1 ORDER BY ord", &[&id.to_string()])
        })
        .await
    }

    async fn approvals(&self, id: ExecutionId) -> Result<Vec<Approval>> {
        self.run(move |conn| {
            documents(conn, "SELECT doc FROM approvals WHERE execution_id = ?1 ORDER BY ord", &[&id.to_string()])
        })
        .await
    }

    async fn approval(&self, id: ApprovalId) -> Result<Option<Approval>> {
        self.run(move |conn| {
            let doc: Option<String> = conn
                .query_row("SELECT doc FROM approvals WHERE id = ?1", params![id.to_string()], |row| row.get(0))
                .optional()
                .map_err(storage)?;
            doc.map(|doc| parse(&doc)).transpose()
        })
        .await
    }

    async fn acquire_lease(&self, id: ExecutionId, owner: &str, now: Timestamp, ttl_ms: u64) -> Result<bool> {
        let owner = owner.to_string();
        self.run(move |conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
            let held: Option<(String, i64)> = tx
                .query_row(
                    "SELECT owner, expires_at FROM leases WHERE execution_id = ?1",
                    params![id.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(storage)?;
            if let Some((holder, expires)) = held
                && holder != owner
                && expires > now.millis()
            {
                return Ok(false);
            }
            tx.execute(
                "INSERT INTO leases (execution_id, owner, expires_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(execution_id) DO UPDATE SET owner = excluded.owner, expires_at = excluded.expires_at",
                params![id.to_string(), owner, now.plus_millis(ttl_ms).millis()],
            )
            .map_err(storage)?;
            tx.commit().map_err(storage)?;
            Ok(true)
        })
        .await
    }

    async fn release_lease(&self, id: ExecutionId, owner: &str) -> Result<()> {
        let owner = owner.to_string();
        self.run(move |conn| {
            conn.execute("DELETE FROM leases WHERE execution_id = ?1 AND owner = ?2", params![id.to_string(), owner])
                .map_err(storage)?;
            Ok(())
        })
        .await
    }

    async fn release_all_leases(&self) -> Result<u64> {
        self.run(|conn| Ok(conn.execute("DELETE FROM leases", []).map_err(storage)? as u64)).await
    }
}
