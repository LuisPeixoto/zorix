//! The SQLite implementation of [`MemoryStore`] (ADR-0013 to ADR-0015, ADR-0017).
//!
//! It opens its **own connection** to the database file, so a long indexing job never waits for the engine's writes (or
//! the other way round). Both use WAL and a busy timeout. Every query carries the tenant; nothing crosses tenants.

use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use crate::errors::storage;
use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::types::Value as Sql;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params, params_from_iter};
use serde_json::Value;
use zorix_core::{
    AgentId, ChunkView, ConversationId, Corpus, DocState, DocStatus, DocumentWrite, Error, ExecutionId, KnowledgeRoot,
    KnowledgeRootId, MemoryFilter, MemoryId, MemoryItem, MemoryKind, MemoryPatch, MemorySettings, MemoryStatus,
    MemoryStore, NewMemory, NewRoot, OperationRecord, RecallSnapshot, Remembered, Result, Scope, TenantId, TextHit,
    Timestamp, VectorHit, Visibility, chunk_key, memory_key, operation_key, text_hash,
};

/// Most active long-term items per tenant.
pub const MAX_ACTIVE_ITEMS: u64 = 5_000;
/// Most knowledge folders per tenant.
pub const MAX_ROOTS: u64 = 50;
/// Most chunks of one file.
pub const MAX_CHUNKS_PER_FILE: usize = 2_000;
/// Most chunks per tenant.
pub const MAX_CHUNKS_PER_TENANT: u64 = 200_000;
/// Most query terms in one keyword search.
pub const MAX_QUERY_TERMS: usize = 32;
/// Longest query term, in characters.
pub const MAX_TERM_CHARS: usize = 64;
/// Most records of a list when the filter does not say.
const DEFAULT_LIMIT: usize = 200;

/// Stores memory in the same SQLite file as the executions.
#[derive(Clone)]
pub struct SqliteMemoryStore {
    conn: Arc<Mutex<Connection>>,
}

fn invalid(what: &str) -> Error {
    Error::InvalidInput(what.to_string())
}

fn parse<T: serde::de::DeserializeOwned>(text: &str) -> Result<T> {
    serde_json::from_str(text).map_err(|e| Error::Storage(format!("a stored record could not be read: {e}")))
}

fn encode<T: serde::Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(storage)
}

impl SqliteMemoryStore {
    /// Open the database at `path` (creating and migrating it when needed) with its own connection.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Storage(format!("could not create {}: {e}", parent.display())))?;
        }
        Self::prepare(Connection::open(path).map_err(storage)?, true)
    }

    /// A database in memory, for tests that do not need the executions tables to be shared.
    pub fn open_in_memory() -> Result<Self> {
        Self::prepare(Connection::open_in_memory().map_err(storage)?, false)
    }

    fn prepare(mut conn: Connection, file: bool) -> Result<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(storage)?;
        if file {
            conn.pragma_update(None, "journal_mode", "WAL").map_err(storage)?;
            conn.pragma_update(None, "synchronous", "NORMAL").map_err(storage)?;
        }
        // Deleted text is overwritten, not just unlinked: forgetting must mean forgetting.
        conn.pragma_update(None, "secure_delete", "ON").map_err(storage)?;
        crate::schema::migrate(&mut conn).map_err(Error::Storage)?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

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

fn immediate(conn: &mut Connection) -> Result<Transaction<'_>> {
    conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)
}

/// Make the freed pages and the log forget what was deleted.
fn checkpoint(conn: &Connection) {
    let _ = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
}

// ---------------------------------------------------------------------------------------------------------------------
// helpers

/// `(scope = 'global' OR …)` for what a run can see, pushing the values it needs onto `args`.
fn scope_sql(alias: &str, visibility: &Visibility, args: &mut Vec<Sql>) -> String {
    let mut parts = vec![format!("{alias}.scope = 'global'")];
    if let Some(project) = &visibility.project {
        args.push(Sql::Text(project.clone()));
        parts.push(format!("({alias}.scope = 'project' AND {alias}.scope_key = ?{})", args.len()));
    }
    if let Some(conversation) = &visibility.conversation {
        args.push(Sql::Text(conversation.to_string()));
        parts.push(format!("({alias}.scope = 'conversation' AND {alias}.scope_key = ?{})", args.len()));
    }
    if let Some(agent) = &visibility.agent {
        args.push(Sql::Text(agent.to_string()));
        parts.push(format!("({alias}.scope = 'agent' AND {alias}.scope_key = ?{})", args.len()));
    }
    format!("({})", parts.join(" OR "))
}

/// `project AND agent` for the past work a run can see. An agent with memory of its own sees the work nobody owns and its own;
/// every other run sees only the work nobody owns (finding F1 of the Phase 11 audit).
fn operations_sql(alias: &str, visibility: &Visibility, args: &mut Vec<Sql>) -> String {
    let project = match &visibility.project {
        Some(p) => {
            args.push(Sql::Text(p.clone()));
            format!("{alias}.project = ?{}", args.len())
        }
        None => format!("{alias}.project IS NULL"),
    };
    let agent = match &visibility.agent {
        Some(agent) => {
            args.push(Sql::Text(agent.to_string()));
            format!("({alias}.agent IS NULL OR {alias}.agent = ?{})", args.len())
        }
        None => format!("{alias}.agent IS NULL"),
    };
    format!("{project} AND {agent}")
}

/// The FTS5 `MATCH` text for already-split words: each one quoted, joined with OR. Raw text never reaches `MATCH`.
fn match_query(terms: &[String]) -> Option<String> {
    let quoted: Vec<String> = terms
        .iter()
        .map(|t| t.chars().filter(|c| !c.is_control()).take(MAX_TERM_CHARS).collect::<String>())
        .filter(|t| !t.trim().is_empty())
        .take(MAX_QUERY_TERMS)
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect();
    if quoted.is_empty() { None } else { Some(quoted.join(" OR ")) }
}

fn terms_in(text: &str, terms: &[String]) -> u32 {
    let lower = text.to_lowercase();
    terms.iter().filter(|t| !t.is_empty() && lower.contains(&t.to_lowercase())).count() as u32
}

fn blob(vector: &[f32]) -> Vec<u8> {
    vector.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// A score with a total order, so it can sit in a heap.
struct Scored(f32, String);

impl PartialEq for Scored {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Scored {}
impl PartialOrd for Scored {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Scored {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0).then_with(|| other.1.cmp(&self.1))
    }
}

const ITEM_COLUMNS: &str = "id, tenant, scope, scope_key, kind, subject, content, status, superseded_by, origin, created_at, updated_at, confirmed_at, valid_until, last_used_at, use_count";

fn item_from(row: &Row<'_>) -> rusqlite::Result<MemoryItem> {
    let id: String = row.get(0)?;
    let tenant: String = row.get(1)?;
    let scope: String = row.get(2)?;
    let scope_key: String = row.get(3)?;
    let kind: String = row.get(4)?;
    let status: String = row.get(7)?;
    let superseded_by: Option<String> = row.get(8)?;
    let origin: String = row.get(9)?;
    let bad = |what: &str| rusqlite::Error::InvalidColumnType(0, what.to_string(), rusqlite::types::Type::Text);
    Ok(MemoryItem {
        id: id.parse().map_err(|_| bad("id"))?,
        tenant: TenantId::new(tenant).map_err(|_| bad("tenant"))?,
        scope: Scope::from_parts(&scope, &scope_key).ok_or_else(|| bad("scope"))?,
        kind: MemoryKind::parse(&kind).ok_or_else(|| bad("kind"))?,
        subject: row.get(5)?,
        content: row.get(6)?,
        status: if status == "active" { MemoryStatus::Active } else { MemoryStatus::Superseded },
        superseded_by: superseded_by.map(|s| s.parse()).transpose().map_err(|_| bad("superseded_by"))?,
        origin: serde_json::from_str(&origin).map_err(|_| bad("origin"))?,
        created_at: Timestamp(row.get(10)?),
        updated_at: Timestamp(row.get(11)?),
        confirmed_at: Timestamp(row.get(12)?),
        valid_until: row.get::<_, Option<i64>>(13)?.map(Timestamp),
        last_used_at: row.get::<_, Option<i64>>(14)?.map(Timestamp),
        use_count: row.get(15)?,
    })
}

fn load_item(tx: &Connection, tenant: &TenantId, id: MemoryId) -> Result<Option<MemoryItem>> {
    tx.query_row(
        &format!("SELECT {ITEM_COLUMNS} FROM memory_items WHERE id = ?1 AND tenant = ?2"),
        params![id.to_string(), tenant.as_str()],
        item_from,
    )
    .optional()
    .map_err(storage)
}

fn active_count(tx: &Connection, tenant: &TenantId) -> Result<u64> {
    tx.query_row(
        "SELECT COUNT(*) FROM memory_items WHERE tenant = ?1 AND status = 'active'",
        params![tenant.as_str()],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n as u64)
    .map_err(storage)
}

/// Remove `keys` from every frozen snapshot that holds them, and forget the references.
fn scrub(tx: &Connection, keys: &[String]) -> Result<()> {
    if keys.is_empty() {
        return Ok(());
    }
    let mut affected: HashSet<String> = HashSet::new();
    {
        let mut find = tx.prepare_cached("SELECT execution_id FROM snapshot_items WHERE item = ?1").map_err(storage)?;
        for key in keys {
            let rows = find.query_map(params![key], |r| r.get::<_, String>(0)).map_err(storage)?;
            for row in rows {
                affected.insert(row.map_err(storage)?);
            }
        }
    }
    for execution in affected {
        let doc: Option<String> = tx
            .query_row("SELECT doc FROM context_snapshots WHERE execution_id = ?1", params![execution], |r| r.get(0))
            .optional()
            .map_err(storage)?;
        let Some(doc) = doc else { continue };
        let mut value: Value = parse(&doc)?;
        if let Some(items) = value.get_mut("items").and_then(Value::as_array_mut) {
            items.retain(|item| item.get("id").and_then(Value::as_str).is_none_or(|id| !keys.iter().any(|k| k == id)));
        }
        if let Some(object) = value.as_object_mut() {
            object.insert("scrubbed".into(), Value::Bool(true));
        }
        tx.execute(
            "UPDATE context_snapshots SET doc = ?1 WHERE execution_id = ?2",
            params![encode(&value)?, execution],
        )
        .map_err(storage)?;
    }
    for key in keys {
        tx.execute("DELETE FROM snapshot_items WHERE item = ?1", params![key]).map_err(storage)?;
    }
    Ok(())
}

fn root_from(row: &Row<'_>) -> rusqlite::Result<KnowledgeRoot> {
    let bad = |what: &str| rusqlite::Error::InvalidColumnType(0, what.to_string(), rusqlite::types::Type::Text);
    let id: String = row.get(0)?;
    let tenant: String = row.get(1)?;
    let scope: String = row.get(2)?;
    let scope_key: String = row.get(3)?;
    let include: String = row.get(5)?;
    let exclude: String = row.get(6)?;
    let last_sync: Option<String> = row.get(12)?;
    Ok(KnowledgeRoot {
        id: id.parse().map_err(|_| bad("id"))?,
        tenant: TenantId::new(tenant).map_err(|_| bad("tenant"))?,
        scope: Scope::from_parts(&scope, &scope_key).ok_or_else(|| bad("scope"))?,
        path: row.get(4)?,
        include: serde_json::from_str(&include).map_err(|_| bad("include"))?,
        exclude: serde_json::from_str(&exclude).map_err(|_| bad("exclude"))?,
        max_file_bytes: row.get::<_, i64>(7)? as u64,
        enabled: row.get::<_, i64>(8)? != 0,
        auto_sync: row.get::<_, i64>(9)? != 0,
        created_at: Timestamp(row.get(10)?),
        last_sync_at: row.get::<_, Option<i64>>(11)?.map(Timestamp),
        last_sync: last_sync.map(|s| serde_json::from_str(&s)).transpose().map_err(|_| bad("last_sync"))?,
    })
}

const ROOT_COLUMNS: &str = "id, tenant, scope, scope_key, path, include, exclude, max_file_bytes, enabled, auto_sync, created_at, last_sync_at, last_sync";

/// The keys of every chunk of a root, for scrubbing snapshots.
fn chunk_keys_of_document(tx: &Connection, document: &str) -> Result<Vec<String>> {
    let mut stmt = tx.prepare("SELECT id FROM knowledge_chunks WHERE document_id = ?1").map_err(storage)?;
    let rows = stmt.query_map(params![document], |r| r.get::<_, i64>(0)).map_err(storage)?;
    rows.map(|r| r.map(chunk_key).map_err(storage)).collect()
}

fn chunk_keys_of_root(tx: &Connection, root: KnowledgeRootId) -> Result<Vec<String>> {
    let mut stmt = tx
        .prepare(
            "SELECT c.id FROM knowledge_chunks c JOIN knowledge_documents d ON d.id = c.document_id WHERE d.root_id = ?1",
        )
        .map_err(storage)?;
    let rows = stmt.query_map(params![root.to_string()], |r| r.get::<_, i64>(0)).map_err(storage)?;
    rows.map(|r| r.map(chunk_key).map_err(storage)).collect()
}

fn owns_root(tx: &Connection, tenant: &TenantId, root: KnowledgeRootId) -> Result<bool> {
    tx.query_row(
        "SELECT 1 FROM knowledge_roots WHERE id = ?1 AND tenant = ?2",
        params![root.to_string(), tenant.as_str()],
        |_| Ok(()),
    )
    .optional()
    .map(|r| r.is_some())
    .map_err(storage)
}

fn collect_top(
    conn: &Connection,
    sql: &str,
    args: Vec<Sql>,
    query: &[f32],
    limit: usize,
    corpus: Corpus,
) -> Result<Vec<VectorHit>> {
    let mut stmt = conn.prepare(sql).map_err(storage)?;
    let mut rows = stmt.query(params_from_iter(args)).map_err(storage)?;
    let mut heap: BinaryHeap<Reverse<Scored>> = BinaryHeap::new();
    while let Some(row) = rows.next().map_err(storage)? {
        let id: String = row.get(0).map_err(storage)?;
        let bytes: Vec<u8> = row.get(1).map_err(storage)?;
        if bytes.len() != query.len() * 4 {
            continue; // a vector of another size is never compared
        }
        let score: f32 = bytes.as_chunks::<4>().0.iter().zip(query).map(|(b, q)| f32::from_le_bytes(*b) * q).sum();
        heap.push(Reverse(Scored(score, id)));
        if heap.len() > limit {
            heap.pop();
        }
    }
    let mut best: Vec<Scored> = heap.into_iter().map(|Reverse(s)| s).collect();
    best.sort_by(|a, b| b.cmp(a));
    Ok(best
        .into_iter()
        .enumerate()
        .map(|(i, Scored(cosine, id))| VectorHit { corpus, id, rank: i as u32 + 1, cosine })
        .collect())
}

// ---------------------------------------------------------------------------------------------------------------------

#[async_trait]
impl MemoryStore for SqliteMemoryStore {
    // ---- long-term items ----

    async fn remember(&self, tenant: &TenantId, new: NewMemory, now: Timestamp) -> Result<Remembered> {
        new.validate()?;
        let tenant = tenant.clone();
        self.run(move |conn| {
            let tx = immediate(conn)?;
            let content = new.content.trim().to_string();
            let hash = text_hash(&content);
            let (scope, scope_key) = new.scope.parts();
            // The same text already in force changes nothing.
            let existing: Option<String> = tx
                .query_row(
                    "SELECT id FROM memory_items WHERE tenant = ?1 AND scope = ?2 AND scope_key = ?3 AND content_hash = ?4 AND status = 'active'",
                    params![tenant.as_str(), scope, scope_key, hash],
                    |r| r.get(0),
                )
                .optional()
                .map_err(storage)?;
            if let Some(id) = existing {
                let id: MemoryId = id.parse().map_err(|_| Error::Storage("a stored id is not valid".into()))?;
                let item = load_item(&tx, &tenant, id)?.ok_or_else(|| Error::Storage("an item vanished".into()))?;
                return Ok(Remembered::Duplicate(item));
            }
            // What this replaces: the ids named, and whatever is in force about the same subject in the same scope.
            let mut old: Vec<MemoryId> = Vec::new();
            for id in &new.replaces {
                let item = load_item(&tx, &tenant, *id)?.ok_or_else(|| invalid("A memory to replace was not found."))?;
                if item.status != MemoryStatus::Active {
                    return Err(invalid("A memory to replace is not in force any more."));
                }
                if !old.contains(id) {
                    old.push(*id);
                }
            }
            if let Some(subject) = new.subject.as_deref().map(str::trim) {
                let mut stmt = tx
                    .prepare("SELECT id FROM memory_items WHERE tenant = ?1 AND scope = ?2 AND scope_key = ?3 AND subject = ?4 AND status = 'active'")
                    .map_err(storage)?;
                let ids = stmt.query_map(params![tenant.as_str(), scope, scope_key, subject], |r| r.get::<_, String>(0)).map_err(storage)?;
                for id in ids {
                    let id: MemoryId = id.map_err(storage)?.parse().map_err(|_| Error::Storage("a stored id is not valid".into()))?;
                    if !old.contains(&id) {
                        old.push(id);
                    }
                }
            }
            if active_count(&tx, &tenant)? + 1 > MAX_ACTIVE_ITEMS + old.len() as u64 {
                return Err(invalid("There are too many memories (5000). Forget some first."));
            }
            let id = MemoryId::new();
            tx.execute(
                "INSERT INTO memory_items (id, tenant, scope, scope_key, kind, subject, content, content_hash, status, superseded_by, origin, created_at, updated_at, confirmed_at, valid_until, use_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'active', NULL, ?9, ?10, ?10, ?10, ?11, 0)",
                params![
                    id.to_string(),
                    tenant.as_str(),
                    scope,
                    scope_key,
                    new.kind.as_str(),
                    new.subject.as_deref().map(str::trim),
                    content,
                    hash,
                    encode(&new.origin)?,
                    now.millis(),
                    new.valid_until.map(|t| t.millis()),
                ],
            )
            .map_err(storage)?;
            for previous in &old {
                tx.execute(
                    "UPDATE memory_items SET status = 'superseded', superseded_by = ?1, updated_at = ?2 WHERE id = ?3 AND tenant = ?4",
                    params![id.to_string(), now.millis(), previous.to_string(), tenant.as_str()],
                )
                .map_err(storage)?;
            }
            let item = load_item(&tx, &tenant, id)?.ok_or_else(|| Error::Storage("the new item vanished".into()))?;
            tx.commit().map_err(storage)?;
            Ok(if old.is_empty() { Remembered::Added(item) } else { Remembered::Replaced { new: item, old } })
        })
        .await
    }

    async fn memory(&self, tenant: &TenantId, id: MemoryId) -> Result<Option<MemoryItem>> {
        let tenant = tenant.clone();
        self.run(move |conn| load_item(conn, &tenant, id)).await
    }

    async fn all_memory(&self, tenant: &TenantId, filter: &MemoryFilter, now: Timestamp) -> Result<Vec<MemoryItem>> {
        let (tenant, filter) = (tenant.clone(), filter.clone());
        self.run(move |conn| {
            let mut args: Vec<Sql> = vec![Sql::Text(tenant.to_string())];
            let mut sql = format!("SELECT {ITEM_COLUMNS} FROM memory_items m WHERE m.tenant = ?1");
            if !filter.include_inactive {
                args.push(Sql::Integer(now.millis()));
                sql.push_str(&format!(
                    " AND m.status = 'active' AND (m.valid_until IS NULL OR m.valid_until > ?{})",
                    args.len()
                ));
            }
            if let Some(kind) = filter.kind {
                args.push(Sql::Text(kind.as_str().into()));
                sql.push_str(&format!(" AND m.kind = ?{}", args.len()));
            }
            let limit = if filter.limit == 0 { DEFAULT_LIMIT * 25 } else { filter.limit };
            sql.push_str(&format!(" ORDER BY m.created_at DESC, m.id DESC LIMIT {limit}"));
            let mut stmt = conn.prepare(&sql).map_err(storage)?;
            let rows = stmt.query_map(params_from_iter(args), item_from).map_err(storage)?;
            rows.map(|r| r.map_err(storage)).collect()
        })
        .await
    }

    async fn list_memory(
        &self,
        visibility: &Visibility,
        filter: &MemoryFilter,
        now: Timestamp,
    ) -> Result<Vec<MemoryItem>> {
        let (visibility, filter) = (visibility.clone(), filter.clone());
        self.run(move |conn| {
            let mut args: Vec<Sql> = vec![Sql::Text(visibility.tenant.to_string())];
            let mut sql = format!("SELECT {ITEM_COLUMNS} FROM memory_items m WHERE m.tenant = ?1 AND ");
            sql.push_str(&scope_sql("m", &visibility, &mut args));
            if !filter.include_inactive {
                args.push(Sql::Integer(now.millis()));
                sql.push_str(&format!(
                    " AND m.status = 'active' AND (m.valid_until IS NULL OR m.valid_until > ?{})",
                    args.len()
                ));
            }
            if let Some(kind) = filter.kind {
                args.push(Sql::Text(kind.as_str().into()));
                sql.push_str(&format!(" AND m.kind = ?{}", args.len()));
            }
            if let Some(subject) = &filter.subject {
                args.push(Sql::Text(subject.clone()));
                sql.push_str(&format!(" AND m.subject = ?{}", args.len()));
            }
            let limit = if filter.limit == 0 { DEFAULT_LIMIT } else { filter.limit };
            sql.push_str(&format!(" ORDER BY m.created_at DESC, m.id DESC LIMIT {limit}"));
            let mut stmt = conn.prepare(&sql).map_err(storage)?;
            let rows = stmt.query_map(params_from_iter(args), item_from).map_err(storage)?;
            rows.map(|r| r.map_err(storage)).collect()
        })
        .await
    }

    async fn update_memory(
        &self,
        tenant: &TenantId,
        id: MemoryId,
        patch: MemoryPatch,
        now: Timestamp,
    ) -> Result<MemoryItem> {
        let tenant = tenant.clone();
        self.run(move |conn| {
            let tx = immediate(conn)?;
            let mut item = load_item(&tx, &tenant, id)?.ok_or_else(|| Error::Missing("There is no such memory.".into()))?;
            if item.status != MemoryStatus::Active {
                return Err(invalid("A replaced memory is kept for the record and cannot be edited."));
            }
            if let Some(content) = &patch.content {
                item.content = content.trim().to_string();
            }
            if let Some(subject) = &patch.subject {
                item.subject = subject.as_ref().map(|s| s.trim().to_string());
            }
            if let Some(until) = patch.valid_until {
                item.valid_until = until;
            }
            if let Some(scope) = &patch.scope {
                item.scope = scope.clone();
            }
            NewMemory {
                scope: item.scope.clone(),
                kind: item.kind,
                subject: item.subject.clone(),
                content: item.content.clone(),
                valid_until: item.valid_until,
                origin: item.origin.clone(),
                replaces: vec![],
            }
            .validate()?;
            if patch.confirm {
                item.confirmed_at = now;
            }
            let hash = text_hash(&item.content);
            let (scope, scope_key) = item.scope.parts();
            let clash: Option<String> = tx
                .query_row(
                    "SELECT id FROM memory_items WHERE tenant = ?1 AND scope = ?2 AND scope_key = ?3 AND content_hash = ?4 AND status = 'active' AND id != ?5",
                    params![tenant.as_str(), scope, scope_key, hash, id.to_string()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(storage)?;
            if clash.is_some() {
                return Err(invalid("Another memory in that scope already says the same."));
            }
            tx.execute(
                "UPDATE memory_items SET scope = ?1, scope_key = ?2, subject = ?3, content = ?4, content_hash = ?5, valid_until = ?6, confirmed_at = ?7, updated_at = ?8 WHERE id = ?9 AND tenant = ?10",
                params![
                    scope,
                    scope_key,
                    item.subject,
                    item.content,
                    hash,
                    item.valid_until.map(|t| t.millis()),
                    item.confirmed_at.millis(),
                    now.millis(),
                    id.to_string(),
                    tenant.as_str(),
                ],
            )
            .map_err(storage)?;
            let updated = load_item(&tx, &tenant, id)?.ok_or_else(|| Error::Storage("an item vanished".into()))?;
            tx.commit().map_err(storage)?;
            Ok(updated)
        })
        .await
    }

    async fn forget(&self, tenant: &TenantId, id: MemoryId) -> Result<bool> {
        let tenant = tenant.clone();
        self.run(move |conn| {
            let tx = immediate(conn)?;
            let removed = tx
                .execute("DELETE FROM memory_items WHERE id = ?1 AND tenant = ?2", params![id.to_string(), tenant.as_str()])
                .map_err(storage)?;
            if removed == 0 {
                return Ok(false);
            }
            tx.execute("UPDATE memory_items SET superseded_by = NULL WHERE superseded_by = ?1", params![id.to_string()])
                .map_err(storage)?;
            scrub(&tx, &[memory_key(id)])?;
            // The vector of a text nobody else has goes with it.
            tx.execute(
                "DELETE FROM vectors WHERE text_hash NOT IN (SELECT content_hash FROM memory_items UNION SELECT text_hash FROM knowledge_chunks UNION SELECT text_hash FROM operations)",
                [],
            )
            .map_err(storage)?;
            tx.execute("INSERT INTO memory_fts (memory_fts) VALUES ('optimize')", []).map_err(storage)?;
            tx.commit().map_err(storage)?;
            checkpoint(conn);
            Ok(true)
        })
        .await
    }

    async fn mark_used(&self, tenant: &TenantId, ids: &[MemoryId], now: Timestamp) -> Result<()> {
        let (tenant, ids) = (tenant.clone(), ids.to_vec());
        self.run(move |conn| {
            let tx = immediate(conn)?;
            for id in ids {
                tx.execute(
                    "UPDATE memory_items SET use_count = use_count + 1, last_used_at = ?1 WHERE id = ?2 AND tenant = ?3",
                    params![now.millis(), id.to_string(), tenant.as_str()],
                )
                .map_err(storage)?;
            }
            tx.commit().map_err(storage)
        })
        .await
    }

    // ---- search ----

    async fn text_search(
        &self,
        visibility: &Visibility,
        corpus: Corpus,
        terms: &[String],
        limit: usize,
    ) -> Result<Vec<TextHit>> {
        let Some(matching) = match_query(terms) else { return Ok(Vec::new()) };
        let (visibility, terms) = (visibility.clone(), terms.to_vec());
        self.run(move |conn| {
            let mut args: Vec<Sql> = vec![Sql::Text(matching), Sql::Text(visibility.tenant.to_string())];
            let sql = match corpus {
                Corpus::Memory => {
                    let scope = scope_sql("m", &visibility, &mut args);
                    format!(
                        "SELECT CAST(m.id AS TEXT), -bm25(memory_fts), m.content || ' ' || COALESCE(m.subject, '')
                         FROM memory_fts JOIN memory_items m ON m.rowid = memory_fts.rowid
                         WHERE memory_fts MATCH ?1 AND m.tenant = ?2 AND m.status = 'active' AND {scope}
                         ORDER BY bm25(memory_fts) LIMIT {limit}"
                    )
                }
                Corpus::Knowledge => {
                    let scope = scope_sql("r", &visibility, &mut args);
                    format!(
                        "SELECT CAST(c.id AS TEXT), -bm25(chunks_fts), c.text || ' ' || COALESCE(c.heading, '')
                         FROM chunks_fts JOIN knowledge_chunks c ON c.id = chunks_fts.rowid
                         JOIN knowledge_documents d ON d.id = c.document_id
                         JOIN knowledge_roots r ON r.id = d.root_id
                         WHERE chunks_fts MATCH ?1 AND r.tenant = ?2 AND r.enabled = 1 AND d.status = 'indexed' AND {scope}
                         ORDER BY bm25(chunks_fts) LIMIT {limit}"
                    )
                }
                Corpus::Operations => {
                    let project = operations_sql("o", &visibility, &mut args);
                    format!(
                        "SELECT o.execution_id, -bm25(operations_fts), o.text
                         FROM operations_fts JOIN operations o ON o.rowid = operations_fts.rowid
                         WHERE operations_fts MATCH ?1 AND o.tenant = ?2 AND {project}
                         ORDER BY bm25(operations_fts) LIMIT {limit}"
                    )
                }
            };
            let mut stmt = conn.prepare(&sql).map_err(storage)?;
            let rows = stmt
                .query_map(params_from_iter(args), |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?, r.get::<_, String>(2)?)))
                .map_err(storage)?;
            let mut hits = Vec::new();
            for (index, row) in rows.enumerate() {
                let (id, score, text) = row.map_err(storage)?;
                hits.push(TextHit { corpus, id, rank: index as u32 + 1, score, matched_terms: terms_in(&text, &terms) });
            }
            Ok(hits)
        })
        .await
    }

    async fn vector_search(
        &self,
        visibility: &Visibility,
        corpus: Corpus,
        model: &str,
        query: &[f32],
        limit: usize,
    ) -> Result<Vec<VectorHit>> {
        let (visibility, model, query) = (visibility.clone(), model.to_string(), query.to_vec());
        self.run(move |conn| {
            let mut args: Vec<Sql> = vec![Sql::Text(model), Sql::Text(visibility.tenant.to_string())];
            let sql = match corpus {
                Corpus::Memory => {
                    let scope = scope_sql("m", &visibility, &mut args);
                    format!(
                        "SELECT m.id, v.vector FROM memory_items m JOIN vectors v ON v.model = ?1 AND v.text_hash = m.content_hash
                         WHERE m.tenant = ?2 AND m.status = 'active' AND {scope}"
                    )
                }
                Corpus::Knowledge => {
                    let scope = scope_sql("r", &visibility, &mut args);
                    format!(
                        "SELECT CAST(c.id AS TEXT), v.vector FROM knowledge_chunks c
                         JOIN knowledge_documents d ON d.id = c.document_id
                         JOIN knowledge_roots r ON r.id = d.root_id
                         JOIN vectors v ON v.model = ?1 AND v.text_hash = c.text_hash
                         WHERE r.tenant = ?2 AND r.enabled = 1 AND d.status = 'indexed' AND {scope}"
                    )
                }
                Corpus::Operations => {
                    let project = operations_sql("o", &visibility, &mut args);
                    format!(
                        "SELECT o.execution_id, v.vector FROM operations o JOIN vectors v ON v.model = ?1 AND v.text_hash = o.text_hash
                         WHERE o.tenant = ?2 AND {project}"
                    )
                }
            };
            collect_top(conn, &sql, args, &query, limit, corpus)
        })
        .await
    }

    // ---- vectors ----

    async fn missing_vectors(&self, model: &str, hashes: &[String]) -> Result<Vec<String>> {
        let (model, hashes) = (model.to_string(), hashes.to_vec());
        self.run(move |conn| {
            let mut have: HashSet<String> = HashSet::new();
            for batch in hashes.chunks(400) {
                let marks = vec!["?"; batch.len()].join(",");
                let mut stmt = conn
                    .prepare(&format!("SELECT text_hash FROM vectors WHERE model = ?1 AND text_hash IN ({marks})"))
                    .map_err(storage)?;
                let mut args: Vec<Sql> = vec![Sql::Text(model.clone())];
                args.extend(batch.iter().cloned().map(Sql::Text));
                let rows = stmt.query_map(params_from_iter(args), |r| r.get::<_, String>(0)).map_err(storage)?;
                for row in rows {
                    have.insert(row.map_err(storage)?);
                }
            }
            let mut seen = HashSet::new();
            Ok(hashes.into_iter().filter(|h| !have.contains(h) && seen.insert(h.clone())).collect())
        })
        .await
    }

    async fn put_vectors(&self, model: &str, vectors: Vec<(String, Vec<f32>)>) -> Result<()> {
        let model = model.to_string();
        self.run(move |conn| {
            let tx = immediate(conn)?;
            for (hash, vector) in &vectors {
                if vector.is_empty() || vector.iter().any(|f| !f.is_finite()) {
                    return Err(invalid("A vector is empty or has a value that is not a number."));
                }
                tx.execute(
                    "INSERT OR REPLACE INTO vectors (model, text_hash, dims, vector) VALUES (?1, ?2, ?3, ?4)",
                    params![model, hash, vector.len() as i64, blob(vector)],
                )
                .map_err(storage)?;
            }
            tx.commit().map_err(storage)
        })
        .await
    }

    async fn gc_vectors(&self, keep: Option<&str>) -> Result<u64> {
        let keep = keep.map(str::to_string);
        self.run(move |conn| {
            let tx = immediate(conn)?;
            let mut removed = 0usize;
            if let Some(keep) = &keep {
                removed += tx.execute("DELETE FROM vectors WHERE model != ?1", params![keep]).map_err(storage)?;
            }
            removed += tx
                .execute(
                    "DELETE FROM vectors WHERE text_hash NOT IN (SELECT content_hash FROM memory_items UNION SELECT text_hash FROM knowledge_chunks UNION SELECT text_hash FROM operations)",
                    [],
                )
                .map_err(storage)?;
            tx.commit().map_err(storage)?;
            Ok(removed as u64)
        })
        .await
    }

    async fn texts_without_vector(
        &self,
        tenant: &TenantId,
        model: &str,
        limit: usize,
    ) -> Result<Vec<(String, String)>> {
        let (tenant, model) = (tenant.clone(), model.to_string());
        self.run(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT m.content_hash, m.content FROM memory_items m
                       WHERE m.tenant = ?1 AND m.status = 'active' AND NOT EXISTS (SELECT 1 FROM vectors v WHERE v.model = ?2 AND v.text_hash = m.content_hash)
                     UNION
                     SELECT o.text_hash, o.text FROM operations o
                       WHERE o.tenant = ?1 AND NOT EXISTS (SELECT 1 FROM vectors v WHERE v.model = ?2 AND v.text_hash = o.text_hash)
                     LIMIT ?3",
                )
                .map_err(storage)?;
            let rows = stmt
                .query_map(params![tenant.as_str(), model, limit as i64], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .map_err(storage)?;
            rows.map(|r| r.map_err(storage)).collect()
        })
        .await
    }

    async fn vector_coverage(&self, tenant: &TenantId, model: &str) -> Result<(u64, u64)> {
        let (tenant, model) = (tenant.clone(), model.to_string());
        self.run(move |conn| {
            let hashes = "SELECT m.content_hash AS h FROM memory_items m WHERE m.tenant = ?1 AND m.status = 'active'
                 UNION SELECT c.text_hash FROM knowledge_chunks c JOIN knowledge_documents d ON d.id = c.document_id JOIN knowledge_roots r ON r.id = d.root_id WHERE r.tenant = ?1
                 UNION SELECT o.text_hash FROM operations o WHERE o.tenant = ?1";
            let total: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM ({hashes})"), params![tenant.as_str()], |r| r.get(0))
                .map_err(storage)?;
            let covered: i64 = conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM ({hashes}) x JOIN vectors v ON v.model = ?2 AND v.text_hash = x.h"),
                    params![tenant.as_str(), model],
                    |r| r.get(0),
                )
                .map_err(storage)?;
            Ok((covered as u64, total as u64))
        })
        .await
    }

    // ---- knowledge ----

    async fn add_root(&self, tenant: &TenantId, root: NewRoot, now: Timestamp) -> Result<KnowledgeRoot> {
        if matches!(root.scope, Scope::Conversation(_)) {
            return Err(invalid("A folder is global or tied to a project."));
        }
        let tenant = tenant.clone();
        self.run(move |conn| {
            let tx = immediate(conn)?;
            let count: i64 = tx
                .query_row("SELECT COUNT(*) FROM knowledge_roots WHERE tenant = ?1", params![tenant.as_str()], |r| r.get(0))
                .map_err(storage)?;
            if count as u64 >= MAX_ROOTS {
                return Err(invalid("There are too many folders (50). Remove one first."));
            }
            let exists: Option<i64> = tx
                .query_row("SELECT 1 FROM knowledge_roots WHERE tenant = ?1 AND path = ?2", params![tenant.as_str(), root.path], |r| r.get(0))
                .optional()
                .map_err(storage)?;
            if exists.is_some() {
                return Err(invalid("That folder is already added."));
            }
            let id = KnowledgeRootId::new();
            let (scope, scope_key) = root.scope.parts();
            tx.execute(
                "INSERT INTO knowledge_roots (id, tenant, scope, scope_key, path, include, exclude, max_file_bytes, enabled, auto_sync, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9, ?10)",
                params![
                    id.to_string(),
                    tenant.as_str(),
                    scope,
                    scope_key,
                    root.path,
                    encode(&root.include)?,
                    encode(&root.exclude)?,
                    root.max_file_bytes as i64,
                    root.auto_sync as i64,
                    now.millis(),
                ],
            )
            .map_err(storage)?;
            let added = tx
                .query_row(&format!("SELECT {ROOT_COLUMNS} FROM knowledge_roots WHERE id = ?1"), params![id.to_string()], root_from)
                .map_err(storage)?;
            tx.commit().map_err(storage)?;
            Ok(added)
        })
        .await
    }

    async fn roots(&self, visibility: &Visibility) -> Result<Vec<KnowledgeRoot>> {
        let visibility = visibility.clone();
        self.run(move |conn| {
            let mut args: Vec<Sql> = vec![Sql::Text(visibility.tenant.to_string())];
            let scope = scope_sql("r", &visibility, &mut args);
            let sql = format!("SELECT {ROOT_COLUMNS} FROM knowledge_roots r WHERE r.tenant = ?1 AND {scope} ORDER BY r.created_at, r.id");
            let mut stmt = conn.prepare(&sql).map_err(storage)?;
            let rows = stmt.query_map(params_from_iter(args), root_from).map_err(storage)?;
            rows.map(|r| r.map_err(storage)).collect()
        })
        .await
    }

    async fn all_roots(&self, tenant: &TenantId) -> Result<Vec<KnowledgeRoot>> {
        let tenant = tenant.clone();
        self.run(move |conn| {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {ROOT_COLUMNS} FROM knowledge_roots WHERE tenant = ?1 ORDER BY created_at, id"
                ))
                .map_err(storage)?;
            let rows = stmt.query_map(params![tenant.as_str()], root_from).map_err(storage)?;
            rows.map(|r| r.map_err(storage)).collect()
        })
        .await
    }

    async fn update_root(&self, tenant: &TenantId, root: KnowledgeRoot) -> Result<()> {
        let tenant = tenant.clone();
        self.run(move |conn| {
            let changed = conn
                .execute(
                    "UPDATE knowledge_roots SET include = ?1, exclude = ?2, max_file_bytes = ?3, enabled = ?4, auto_sync = ?5 WHERE id = ?6 AND tenant = ?7",
                    params![
                        encode(&root.include)?,
                        encode(&root.exclude)?,
                        root.max_file_bytes as i64,
                        root.enabled as i64,
                        root.auto_sync as i64,
                        root.id.to_string(),
                        tenant.as_str()
                    ],
                )
                .map_err(storage)?;
            if changed == 0 { Err(Error::Missing("There is no such folder.".into())) } else { Ok(()) }
        })
        .await
    }

    async fn finish_sync(&self, tenant: &TenantId, id: KnowledgeRootId, at: Timestamp, summary: Value) -> Result<()> {
        let tenant = tenant.clone();
        self.run(move |conn| {
            conn.execute(
                "UPDATE knowledge_roots SET last_sync_at = ?1, last_sync = ?2 WHERE id = ?3 AND tenant = ?4",
                params![at.millis(), summary.to_string(), id.to_string(), tenant.as_str()],
            )
            .map_err(storage)?;
            Ok(())
        })
        .await
    }

    async fn remove_root(&self, tenant: &TenantId, id: KnowledgeRootId) -> Result<bool> {
        let tenant = tenant.clone();
        self.run(move |conn| {
            let tx = immediate(conn)?;
            if !owns_root(&tx, &tenant, id)? {
                return Ok(false);
            }
            scrub(&tx, &chunk_keys_of_root(&tx, id)?)?;
            tx.execute(
                "DELETE FROM knowledge_chunks WHERE document_id IN (SELECT id FROM knowledge_documents WHERE root_id = ?1)",
                params![id.to_string()],
            )
            .map_err(storage)?;
            tx.execute("DELETE FROM knowledge_documents WHERE root_id = ?1", params![id.to_string()]).map_err(storage)?;
            tx.execute("DELETE FROM knowledge_roots WHERE id = ?1", params![id.to_string()]).map_err(storage)?;
            tx.execute(
                "DELETE FROM vectors WHERE text_hash NOT IN (SELECT content_hash FROM memory_items UNION SELECT text_hash FROM knowledge_chunks UNION SELECT text_hash FROM operations)",
                [],
            )
            .map_err(storage)?;
            tx.execute("INSERT INTO chunks_fts (chunks_fts) VALUES ('optimize')", []).map_err(storage)?;
            tx.commit().map_err(storage)?;
            checkpoint(conn);
            Ok(true)
        })
        .await
    }

    async fn touch_document(&self, root: KnowledgeRootId, rel_path: &str, size: u64, mtime_ns: i64) -> Result<()> {
        let rel_path = rel_path.to_string();
        self.run(move |conn| {
            conn.execute(
                "UPDATE knowledge_documents SET size = ?3, mtime_ns = ?4 WHERE root_id = ?1 AND rel_path = ?2",
                params![root.to_string(), rel_path, size as i64, mtime_ns],
            )
            .map_err(storage)?;
            Ok(())
        })
        .await
    }

    async fn document_states(&self, root: KnowledgeRootId) -> Result<HashMap<String, DocState>> {
        self.run(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT rel_path, size, mtime_ns, content_hash, status FROM knowledge_documents WHERE root_id = ?1",
                )
                .map_err(storage)?;
            let rows = stmt
                .query_map(params![root.to_string()], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                })
                .map_err(storage)?;
            let mut map = HashMap::new();
            for row in rows {
                let (path, size, mtime_ns, content_hash, status) = row.map_err(storage)?;
                let status =
                    DocStatus::parse(&status).ok_or_else(|| Error::Storage("a stored status is not valid".into()))?;
                map.insert(path, DocState { size: size as u64, mtime_ns, content_hash, status });
            }
            Ok(map)
        })
        .await
    }

    async fn write_document(&self, write: DocumentWrite) -> Result<()> {
        if write.chunks.len() > MAX_CHUNKS_PER_FILE {
            return Err(invalid("A file has too many pieces (2000 at most)."));
        }
        self.run(move |conn| {
            let tx = immediate(conn)?;
            let (tenant, _): (String, i64) = tx
                .query_row("SELECT tenant, 1 FROM knowledge_roots WHERE id = ?1", params![write.root.to_string()], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()
                .map_err(storage)?
                .ok_or_else(|| Error::Missing("There is no such folder.".into()))?;
            let existing: Option<(String, i64)> = tx
                .query_row(
                    "SELECT id, chunks FROM knowledge_documents WHERE root_id = ?1 AND rel_path = ?2",
                    params![write.root.to_string(), write.rel_path],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(storage)?;
            let replaced = existing.as_ref().map(|(_, n)| *n as u64).unwrap_or(0);
            let total: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM knowledge_chunks c JOIN knowledge_documents d ON d.id = c.document_id JOIN knowledge_roots r ON r.id = d.root_id WHERE r.tenant = ?1",
                    params![tenant],
                    |r| r.get(0),
                )
                .map_err(storage)?;
            if (total as u64).saturating_sub(replaced) + write.chunks.len() as u64 > MAX_CHUNKS_PER_TENANT {
                return Err(invalid("There are too many indexed pieces (200000). Remove a folder first."));
            }
            let document = match existing {
                Some((id, _)) => {
                    // The old pieces go like a removed file's do: out of the frozen recall of past runs too, so removing the folder
                    // later leaves no old version of its text behind, and no deleted id is left referenced for a new piece to reuse.
                    scrub(&tx, &chunk_keys_of_document(&tx, &id)?)?;
                    tx.execute("DELETE FROM knowledge_chunks WHERE document_id = ?1", params![id]).map_err(storage)?;
                    id
                }
                None => uuid_like(&write),
            };
            tx.execute(
                "INSERT INTO knowledge_documents (id, root_id, rel_path, kind, size, mtime_ns, content_hash, status, error, chunks, indexed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT (root_id, rel_path) DO UPDATE SET kind = excluded.kind, size = excluded.size, mtime_ns = excluded.mtime_ns,
                   content_hash = excluded.content_hash, status = excluded.status, error = excluded.error, chunks = excluded.chunks,
                   indexed_at = excluded.indexed_at",
                params![
                    document,
                    write.root.to_string(),
                    write.rel_path,
                    write.kind,
                    write.state.size as i64,
                    write.state.mtime_ns,
                    write.state.content_hash,
                    write.state.status.as_str(),
                    write.error,
                    write.chunks.len() as i64,
                    write.indexed_at.millis(),
                ],
            )
            .map_err(storage)?;
            {
                let mut insert = tx
                    .prepare_cached(
                        "INSERT INTO knowledge_chunks (document_id, ord, text, text_hash, heading, line_start, line_end, page, tokens)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    )
                    .map_err(storage)?;
                for (ord, chunk) in write.chunks.iter().enumerate() {
                    insert
                        .execute(params![
                            document,
                            ord as i64,
                            chunk.text,
                            chunk.text_hash,
                            chunk.heading,
                            chunk.line_start,
                            chunk.line_end,
                            chunk.page,
                            chunk.tokens
                        ])
                        .map_err(storage)?;
                }
            }
            tx.commit().map_err(storage)
        })
        .await
    }

    async fn remove_documents(&self, root: KnowledgeRootId, rel_paths: &[String]) -> Result<()> {
        let rel_paths = rel_paths.to_vec();
        self.run(move |conn| {
            let tx = immediate(conn)?;
            for rel in &rel_paths {
                let id: Option<String> = tx
                    .query_row(
                        "SELECT id FROM knowledge_documents WHERE root_id = ?1 AND rel_path = ?2",
                        params![root.to_string(), rel],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(storage)?;
                let Some(id) = id else { continue };
                scrub(&tx, &chunk_keys_of_document(&tx, &id)?)?;
                tx.execute("DELETE FROM knowledge_chunks WHERE document_id = ?1", params![id]).map_err(storage)?;
                tx.execute("DELETE FROM knowledge_documents WHERE id = ?1", params![id]).map_err(storage)?;
            }
            tx.commit().map_err(storage)
        })
        .await
    }

    async fn problem_documents(&self, root: KnowledgeRootId) -> Result<Vec<(String, DocStatus, String)>> {
        self.run(move |conn| {
            let mut stmt = conn
                .prepare("SELECT rel_path, status, COALESCE(error, '') FROM knowledge_documents WHERE root_id = ?1 AND status != 'indexed' ORDER BY rel_path")
                .map_err(storage)?;
            let rows = stmt
                .query_map(params![root.to_string()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
                .map_err(storage)?;
            let mut out = Vec::new();
            for row in rows {
                let (path, status, error) = row.map_err(storage)?;
                if let Some(status) = DocStatus::parse(&status) {
                    out.push((path, status, error));
                }
            }
            Ok(out)
        })
        .await
    }

    async fn chunks(&self, tenant: &TenantId, ids: &[i64]) -> Result<Vec<ChunkView>> {
        let (tenant, ids) = (tenant.clone(), ids.to_vec());
        self.run(move |conn| {
            let mut out = Vec::new();
            let mut stmt = conn
                .prepare(
                    "SELECT c.id, r.id, r.path, d.rel_path, c.ord, c.text, c.text_hash, c.heading, c.line_start, c.line_end, c.page, c.tokens, d.indexed_at
                     FROM knowledge_chunks c JOIN knowledge_documents d ON d.id = c.document_id JOIN knowledge_roots r ON r.id = d.root_id
                     WHERE c.id = ?1 AND r.tenant = ?2",
                )
                .map_err(storage)?;
            for id in ids {
                let row = stmt
                    .query_row(params![id, tenant.as_str()], |r| {
                        let root: String = r.get(1)?;
                        Ok(ChunkView {
                            id: r.get(0)?,
                            root: root.parse().map_err(|_| rusqlite::Error::InvalidColumnType(1, "root".into(), rusqlite::types::Type::Text))?,
                            root_path: r.get(2)?,
                            rel_path: r.get(3)?,
                            ord: r.get(4)?,
                            text: r.get(5)?,
                            text_hash: r.get(6)?,
                            heading: r.get(7)?,
                            line_start: r.get(8)?,
                            line_end: r.get(9)?,
                            page: r.get(10)?,
                            tokens: r.get(11)?,
                            indexed_at: Timestamp(r.get(12)?),
                        })
                    })
                    .optional()
                    .map_err(storage)?;
                out.extend(row);
            }
            Ok(out)
        })
        .await
    }

    async fn chunk_hashes_without_vector(
        &self,
        root: KnowledgeRootId,
        model: &str,
        limit: usize,
    ) -> Result<Vec<(String, String)>> {
        let model = model.to_string();
        self.run(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT c.text_hash, c.text FROM knowledge_chunks c JOIN knowledge_documents d ON d.id = c.document_id
                     WHERE d.root_id = ?1 AND NOT EXISTS (SELECT 1 FROM vectors v WHERE v.model = ?2 AND v.text_hash = c.text_hash)
                     GROUP BY c.text_hash ORDER BY MIN(c.id) LIMIT ?3",
                )
                .map_err(storage)?;
            let rows = stmt
                .query_map(params![root.to_string(), model, limit as i64], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .map_err(storage)?;
            rows.map(|r| r.map_err(storage)).collect()
        })
        .await
    }

    // ---- operational ----

    async fn record_operation(&self, record: OperationRecord) -> Result<()> {
        self.run(move |conn| {
            let tx = immediate(conn)?;
            // Not INSERT OR REPLACE: the delete trigger must run to keep the full-text index in step.
            tx.execute("DELETE FROM operations WHERE execution_id = ?1", params![record.execution_id.to_string()]).map_err(storage)?;
            tx.execute(
                "INSERT INTO operations (execution_id, tenant, project, conversation, state, finished_at, text, text_hash, doc, agent)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    record.execution_id.to_string(),
                    record.tenant.as_str(),
                    record.project,
                    record.conversation.as_ref().map(ConversationId::to_string),
                    record.state,
                    record.finished_at.millis(),
                    record.text,
                    text_hash(record.text.trim()),
                    record.doc.to_string(),
                    record.agent.as_ref().map(AgentId::to_string),
                ],
            )
            .map_err(storage)?;
            tx.commit().map_err(storage)
        })
        .await
    }

    async fn operation(&self, tenant: &TenantId, id: ExecutionId) -> Result<Option<OperationRecord>> {
        let tenant = tenant.clone();
        self.run(move |conn| {
            conn.query_row(
                "SELECT project, conversation, state, finished_at, text, doc, agent FROM operations WHERE execution_id = ?1 AND tenant = ?2",
                params![id.to_string(), tenant.as_str()],
                |r| {
                    let conversation: Option<String> = r.get(1)?;
                    let doc: String = r.get(5)?;
                    let agent: Option<String> = r.get(6)?;
                    Ok(OperationRecord {
                        execution_id: id,
                        tenant: tenant.clone(),
                        project: r.get(0)?,
                        conversation: conversation.and_then(|c| ConversationId::new(c).ok()),
                        agent: agent.and_then(|a| a.parse::<AgentId>().ok()),
                        state: r.get(2)?,
                        finished_at: Timestamp(r.get(3)?),
                        text: r.get(4)?,
                        doc: serde_json::from_str(&doc).unwrap_or(Value::Null),
                    })
                },
            )
            .optional()
            .map_err(storage)
        })
        .await
    }

    async fn operations_missing(&self, limit: usize) -> Result<Vec<ExecutionId>> {
        self.run(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT e.id FROM executions e LEFT JOIN operations o ON o.execution_id = e.id
                     WHERE e.state IN ('completed', 'failed', 'cancelled') AND o.execution_id IS NULL
                     ORDER BY e.created_at, e.id LIMIT ?1",
                )
                .map_err(storage)?;
            let rows = stmt.query_map(params![limit as i64], |r| r.get::<_, String>(0)).map_err(storage)?;
            let mut out = Vec::new();
            for row in rows {
                if let Ok(id) = row.map_err(storage)?.parse() {
                    out.push(id);
                }
            }
            Ok(out)
        })
        .await
    }

    async fn forget_operation(&self, tenant: &TenantId, id: ExecutionId) -> Result<bool> {
        let tenant = tenant.clone();
        self.run(move |conn| {
            let tx = immediate(conn)?;
            let removed = tx
                .execute("DELETE FROM operations WHERE execution_id = ?1 AND tenant = ?2", params![id.to_string(), tenant.as_str()])
                .map_err(storage)?;
            if removed == 0 {
                return Ok(false);
            }
            scrub(&tx, &[operation_key(id)])?;
            tx.execute(
                "DELETE FROM vectors WHERE text_hash NOT IN (SELECT content_hash FROM memory_items UNION SELECT text_hash FROM knowledge_chunks UNION SELECT text_hash FROM operations)",
                [],
            )
            .map_err(storage)?;
            tx.execute("INSERT INTO operations_fts (operations_fts) VALUES ('optimize')", []).map_err(storage)?;
            tx.commit().map_err(storage)?;
            checkpoint(conn);
            Ok(true)
        })
        .await
    }

    // ---- snapshots and settings ----

    async fn snapshot(&self, id: ExecutionId) -> Result<Option<RecallSnapshot>> {
        self.run(move |conn| read_snapshot(conn, id)).await
    }

    async fn save_snapshot(&self, snapshot: RecallSnapshot) -> Result<RecallSnapshot> {
        self.run(move |conn| {
            let tx = immediate(conn)?;
            let inserted = tx
                .execute(
                    "INSERT OR IGNORE INTO context_snapshots (execution_id, tenant, created_at, doc) VALUES (?1, ?2, ?3, ?4)",
                    params![snapshot.execution_id.to_string(), snapshot.tenant.as_str(), snapshot.created_at.millis(), snapshot.doc.to_string()],
                )
                .map_err(storage)?;
            if inserted == 1 {
                for item in &snapshot.items {
                    tx.execute(
                        "INSERT OR IGNORE INTO snapshot_items (execution_id, item) VALUES (?1, ?2)",
                        params![snapshot.execution_id.to_string(), item],
                    )
                    .map_err(storage)?;
                }
            }
            let stored = read_snapshot(&tx, snapshot.execution_id)?.ok_or_else(|| Error::Storage("a snapshot vanished".into()))?;
            tx.commit().map_err(storage)?;
            Ok(stored)
        })
        .await
    }

    async fn snapshots_using(&self, tenant: &TenantId, item: &str) -> Result<Vec<ExecutionId>> {
        let (tenant, item) = (tenant.clone(), item.to_string());
        self.run(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT s.execution_id FROM snapshot_items i JOIN context_snapshots s ON s.execution_id = i.execution_id
                     WHERE s.tenant = ?1 AND i.item = ?2 ORDER BY s.created_at",
                )
                .map_err(storage)?;
            let rows = stmt.query_map(params![tenant.as_str(), item], |r| r.get::<_, String>(0)).map_err(storage)?;
            let mut out = Vec::new();
            for row in rows {
                if let Ok(id) = row.map_err(storage)?.parse() {
                    out.push(id);
                }
            }
            Ok(out)
        })
        .await
    }

    async fn settings(&self, tenant: &TenantId) -> Result<MemorySettings> {
        let tenant = tenant.clone();
        self.run(move |conn| {
            let doc: Option<String> = conn
                .query_row("SELECT doc FROM memory_settings WHERE tenant = ?1", params![tenant.as_str()], |r| r.get(0))
                .optional()
                .map_err(storage)?;
            match doc {
                Some(doc) => parse(&doc),
                None => Ok(MemorySettings::default()),
            }
        })
        .await
    }

    async fn set_settings(&self, tenant: &TenantId, settings: MemorySettings) -> Result<()> {
        let tenant = tenant.clone();
        self.run(move |conn| {
            conn.execute(
                "INSERT INTO memory_settings (tenant, doc) VALUES (?1, ?2) ON CONFLICT (tenant) DO UPDATE SET doc = excluded.doc",
                params![tenant.as_str(), encode(&settings)?],
            )
            .map_err(storage)?;
            Ok(())
        })
        .await
    }
}

fn read_snapshot(conn: &Connection, id: ExecutionId) -> Result<Option<RecallSnapshot>> {
    let row: Option<(String, i64, String)> = conn
        .query_row(
            "SELECT tenant, created_at, doc FROM context_snapshots WHERE execution_id = ?1",
            params![id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(storage)?;
    let Some((tenant, created_at, doc)) = row else { return Ok(None) };
    let mut stmt =
        conn.prepare("SELECT item FROM snapshot_items WHERE execution_id = ?1 ORDER BY item").map_err(storage)?;
    let items = stmt
        .query_map(params![id.to_string()], |r| r.get::<_, String>(0))
        .map_err(storage)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(storage)?;
    Ok(Some(RecallSnapshot {
        execution_id: id,
        tenant: TenantId::new(tenant).map_err(|e| Error::Storage(e.to_string()))?,
        created_at: Timestamp(created_at),
        items,
        doc: parse(&doc)?,
    }))
}

/// A new id for a document row. It only has to be unique and opaque.
fn uuid_like(write: &DocumentWrite) -> String {
    format!("{}:{}", write.root, text_hash(&write.rel_path).chars().take(16).collect::<String>())
}
