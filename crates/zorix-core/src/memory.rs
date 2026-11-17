//! Memory: what ZORIX keeps between executions, and the contract of the place that keeps it.
//!
//! Four kinds exist (ADR-0013): the conversation (earlier executions of the same conversation), long-term facts, knowledge
//! (indexed documents) and operational records (what was done). Every call takes the tenant, and nothing crosses tenants.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::Result;
use crate::ids::{ApprovalId, ConversationId, ExecutionId, KnowledgeRootId, MemoryId, TenantId, ToolCallId};
use crate::time::Timestamp;

/// SHA-256 of a text, in lowercase hex. It keys vectors, so the same text is never embedded twice, and it detects
/// duplicates. Callers pass the exact text that is embedded (trimmed).
pub fn text_hash(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// SHA-256 of bytes, in lowercase hex (the content hash of a file).
pub fn bytes_hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The key a snapshot uses for a long-term item.
pub fn memory_key(id: MemoryId) -> String {
    format!("m:{id}")
}

/// The key a snapshot uses for a document chunk.
pub fn chunk_key(id: i64) -> String {
    format!("k:{id}")
}

/// The key a snapshot uses for an operational record.
pub fn operation_key(id: ExecutionId) -> String {
    format!("o:{id}")
}

/// Where a memory applies.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Scope {
    /// Everywhere.
    Global,
    /// Runs whose workspace is this folder (a canonical absolute path).
    Project(String),
    /// One conversation.
    Conversation(ConversationId),
}

impl Scope {
    /// The two columns a store keeps: a scope name and its key (empty for `Global`).
    pub fn parts(&self) -> (&'static str, String) {
        match self {
            Scope::Global => ("global", String::new()),
            Scope::Project(path) => ("project", path.clone()),
            Scope::Conversation(id) => ("conversation", id.to_string()),
        }
    }

    /// Rebuild a scope from the two columns.
    pub fn from_parts(name: &str, key: &str) -> Option<Scope> {
        match name {
            "global" => Some(Scope::Global),
            "project" => Some(Scope::Project(key.to_string())),
            "conversation" => ConversationId::new(key).ok().map(Scope::Conversation),
            _ => None,
        }
    }

    /// A short word for people.
    pub fn label(&self) -> String {
        match self {
            Scope::Global => "global".into(),
            Scope::Project(path) => format!("project {path}"),
            Scope::Conversation(id) => format!("conversation {id}"),
        }
    }
}

/// What one execution may see: its tenant, its project and its conversation. Global memory is always visible.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Visibility {
    /// The owner.
    pub tenant: TenantId,
    /// The canonical workspace path, when the run has one.
    pub project: Option<String>,
    /// The conversation, when the run has one.
    pub conversation: Option<ConversationId>,
}

impl Visibility {
    /// Visibility of a tenant with no project and no conversation.
    pub fn tenant(tenant: TenantId) -> Self {
        Self { tenant, project: None, conversation: None }
    }

    /// The scopes visible, widest first.
    pub fn scopes(&self) -> Vec<Scope> {
        let mut scopes = vec![Scope::Global];
        if let Some(project) = &self.project {
            scopes.push(Scope::Project(project.clone()));
        }
        if let Some(conversation) = &self.conversation {
            scopes.push(Scope::Conversation(conversation.clone()));
        }
        scopes
    }

    /// True when `scope` is visible.
    pub fn sees(&self, scope: &Scope) -> bool {
        self.scopes().contains(scope)
    }
}

/// What a long-term item says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    /// Something true ("the project uses Postgres 16").
    Fact,
    /// How a person wants things done ("answer in Portuguese").
    Preference,
}

impl MemoryKind {
    /// The name stored and shown.
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryKind::Fact => "fact",
            MemoryKind::Preference => "preference",
        }
    }

    /// Read a stored name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "fact" => Some(MemoryKind::Fact),
            "preference" => Some(MemoryKind::Preference),
            _ => None,
        }
    }
}

/// Whether an item is in force.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    /// In force.
    Active,
    /// Replaced by a newer item (kept for the record).
    Superseded,
}

/// Who created an item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "snake_case")]
pub enum MemoryOrigin {
    /// A person, by hand.
    User {
        /// Who (a name or `cli` / `desktop`).
        name: String,
    },
    /// The model proposed it and a person approved.
    Model {
        /// The execution that proposed it.
        execution_id: ExecutionId,
        /// The call.
        call_id: ToolCallId,
        /// The approval.
        approval_id: Option<ApprovalId>,
        /// Who approved.
        approved_by: String,
    },
}

/// One long-term fact or preference.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryItem {
    /// Its id.
    pub id: MemoryId,
    /// The owner.
    pub tenant: TenantId,
    /// Where it applies.
    pub scope: Scope,
    /// Fact or preference.
    pub kind: MemoryKind,
    /// What it is about, used to notice when a newer item replaces it.
    pub subject: Option<String>,
    /// The text.
    pub content: String,
    /// In force or replaced.
    pub status: MemoryStatus,
    /// The item that replaced it.
    pub superseded_by: Option<MemoryId>,
    /// Who created it.
    pub origin: MemoryOrigin,
    /// When it was created.
    pub created_at: Timestamp,
    /// When it last changed.
    pub updated_at: Timestamp,
    /// When a person last confirmed it is still true.
    pub confirmed_at: Timestamp,
    /// After this it is no longer used.
    pub valid_until: Option<Timestamp>,
    /// When it last went into a context.
    pub last_used_at: Option<Timestamp>,
    /// How many times it went into a context.
    pub use_count: u32,
}

impl MemoryItem {
    /// Most characters in an item.
    pub const MAX_CONTENT: usize = 500;
    /// Most characters in a subject.
    pub const MAX_SUBJECT: usize = 80;

    /// True when it is in force at `now`.
    pub fn in_force(&self, now: Timestamp) -> bool {
        self.status == MemoryStatus::Active && self.valid_until.is_none_or(|until| until.millis() > now.millis())
    }
}

/// What to remember.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NewMemory {
    /// Where it applies.
    pub scope: Scope,
    /// Fact or preference.
    pub kind: MemoryKind,
    /// What it is about.
    pub subject: Option<String>,
    /// The text.
    pub content: String,
    /// After this it is no longer used.
    pub valid_until: Option<Timestamp>,
    /// Who creates it.
    pub origin: MemoryOrigin,
    /// Items this one replaces.
    pub replaces: Vec<MemoryId>,
}

impl NewMemory {
    /// Check the text and the subject.
    pub fn validate(&self) -> Result<()> {
        let bad = |what: &str| Err(crate::error::Error::InvalidInput(what.to_string()));
        let content = self.content.trim();
        if content.is_empty() {
            return bad("A memory cannot be empty.");
        }
        if content.chars().count() > MemoryItem::MAX_CONTENT {
            return bad("A memory is 500 characters at most.");
        }
        if self.subject.as_ref().is_some_and(|s| s.trim().is_empty() || s.chars().count() > MemoryItem::MAX_SUBJECT) {
            return bad("A subject is 1 to 80 characters.");
        }
        Ok(())
    }
}

/// What happened when something was remembered.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Remembered {
    /// A new item.
    Added(MemoryItem),
    /// The same text was already in force; nothing changed.
    Duplicate(MemoryItem),
    /// A new item that replaced older ones.
    Replaced {
        /// The new item.
        new: MemoryItem,
        /// The items it replaced.
        old: Vec<MemoryId>,
    },
}

impl Remembered {
    /// The item now in force.
    pub fn item(&self) -> &MemoryItem {
        match self {
            Remembered::Added(item) | Remembered::Duplicate(item) => item,
            Remembered::Replaced { new, .. } => new,
        }
    }
}

/// Which items to list.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryFilter {
    /// Also list superseded and expired items.
    pub include_inactive: bool,
    /// Only this kind.
    pub kind: Option<MemoryKind>,
    /// Only items about this subject.
    pub subject: Option<String>,
    /// Most items (0 means 200).
    pub limit: usize,
}

/// A change to an item. `None` leaves a field alone.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MemoryPatch {
    /// New text.
    pub content: Option<String>,
    /// New subject (`Some(None)` clears it).
    pub subject: Option<Option<String>>,
    /// New expiry (`Some(None)` clears it).
    pub valid_until: Option<Option<Timestamp>>,
    /// Move to another scope.
    pub scope: Option<Scope>,
    /// A person says it is still true.
    pub confirm: bool,
}

/// What a search looks through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Corpus {
    /// Long-term items.
    Memory,
    /// Document chunks.
    Knowledge,
    /// Operational records.
    Operations,
}

/// A keyword hit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextHit {
    /// Where it was found.
    pub corpus: Corpus,
    /// The id of the item, chunk or execution.
    pub id: String,
    /// Its place in the list, from 1.
    pub rank: u32,
    /// The BM25 score (smaller is better in SQLite; stored here as a positive number, larger is better).
    pub score: f64,
    /// How many distinct query terms matched.
    pub matched_terms: u32,
}

/// A vector hit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VectorHit {
    /// Where it was found.
    pub corpus: Corpus,
    /// The id of the item, chunk or execution.
    pub id: String,
    /// Its place in the list, from 1.
    pub rank: u32,
    /// Cosine similarity.
    pub cosine: f32,
}

/// What a folder of documents is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeRoot {
    /// Its id.
    pub id: KnowledgeRootId,
    /// The owner.
    pub tenant: TenantId,
    /// `Global` or `Project`.
    pub scope: Scope,
    /// The canonical folder.
    pub path: String,
    /// Globs of files to add to the defaults.
    pub include: Vec<String>,
    /// Globs of files to leave out.
    pub exclude: Vec<String>,
    /// Largest file to read.
    pub max_file_bytes: u64,
    /// Used in searches.
    pub enabled: bool,
    /// Look for changes when the app starts.
    pub auto_sync: bool,
    /// When it was added.
    pub created_at: Timestamp,
    /// When a sync last finished.
    pub last_sync_at: Option<Timestamp>,
    /// The result of the last sync, as JSON.
    pub last_sync: Option<Value>,
}

/// A folder to add.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NewRoot {
    /// `Global` or `Project`.
    pub scope: Scope,
    /// The canonical folder.
    pub path: String,
    /// Globs to add.
    pub include: Vec<String>,
    /// Globs to leave out.
    pub exclude: Vec<String>,
    /// Largest file to read.
    pub max_file_bytes: u64,
    /// Look for changes when the app starts.
    pub auto_sync: bool,
}

/// How a document fared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocStatus {
    /// Indexed.
    Indexed,
    /// Could not be read; the reason is stored.
    Failed,
    /// Left out on purpose (too big, binary, no text).
    Skipped,
}

impl DocStatus {
    /// The name stored.
    pub fn as_str(&self) -> &'static str {
        match self {
            DocStatus::Indexed => "indexed",
            DocStatus::Failed => "failed",
            DocStatus::Skipped => "skipped",
        }
    }

    /// Read a stored name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "indexed" => Some(DocStatus::Indexed),
            "failed" => Some(DocStatus::Failed),
            "skipped" => Some(DocStatus::Skipped),
            _ => None,
        }
    }
}

/// What is known about an indexed file, to tell whether it changed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocState {
    /// Size in bytes.
    pub size: u64,
    /// Modification time, in nanoseconds.
    pub mtime_ns: i64,
    /// SHA-256 of the content, in hex.
    pub content_hash: String,
    /// How it fared.
    pub status: DocStatus,
}

/// One piece of a document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NewChunk {
    /// The text.
    pub text: String,
    /// SHA-256 of the text, in hex (the key of its vector).
    pub text_hash: String,
    /// The heading path, for Markdown.
    pub heading: Option<String>,
    /// First line (1-based), for text files.
    pub line_start: Option<u32>,
    /// Last line, for text files.
    pub line_end: Option<u32>,
    /// The page, for PDFs.
    pub page: Option<u32>,
    /// Estimated tokens.
    pub tokens: u32,
}

/// Everything about one file, written in one transaction (replacing what was there).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DocumentWrite {
    /// The folder.
    pub root: KnowledgeRootId,
    /// The path relative to the folder, with `/`.
    pub rel_path: String,
    /// `markdown`, `text`, `code`, `data`, `html` or `pdf`.
    pub kind: String,
    /// What is known about the file.
    pub state: DocState,
    /// Why it failed or was skipped.
    pub error: Option<String>,
    /// Its chunks (empty unless indexed).
    pub chunks: Vec<NewChunk>,
    /// When it was indexed.
    pub indexed_at: Timestamp,
}

/// A chunk as a search shows it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChunkView {
    /// Its id.
    pub id: i64,
    /// The folder.
    pub root: KnowledgeRootId,
    /// The folder's path.
    pub root_path: String,
    /// The file, relative to the folder.
    pub rel_path: String,
    /// Its position in the file, from 0.
    pub ord: u32,
    /// The text.
    pub text: String,
    /// SHA-256 of the text.
    pub text_hash: String,
    /// The heading path.
    pub heading: Option<String>,
    /// First line.
    pub line_start: Option<u32>,
    /// Last line.
    pub line_end: Option<u32>,
    /// The page.
    pub page: Option<u32>,
    /// Estimated tokens.
    pub tokens: u32,
    /// When the file was indexed.
    pub indexed_at: Timestamp,
}

impl ChunkView {
    /// How to cite it: `path:L10-L42`, `path#page=3` or just the path.
    pub fn citation(&self) -> String {
        match (self.line_start, self.line_end, self.page) {
            (Some(a), Some(b), _) => format!("{}:L{a}-L{b}", self.rel_path),
            (_, _, Some(p)) => format!("{}#page={p}", self.rel_path),
            _ => self.rel_path.clone(),
        }
    }
}

/// A finished execution, as operational memory keeps it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OperationRecord {
    /// The execution.
    pub execution_id: ExecutionId,
    /// The owner.
    pub tenant: TenantId,
    /// The canonical workspace, if any.
    pub project: Option<String>,
    /// The conversation, if any.
    pub conversation: Option<ConversationId>,
    /// `completed`, `failed` or `cancelled`.
    pub state: String,
    /// When it ended.
    pub finished_at: Timestamp,
    /// The text that is searched: prompt, outcome, tools.
    pub text: String,
    /// The structured record.
    pub doc: Value,
}

/// What was retrieved for one execution, frozen so every turn and every resume sends the same bytes (ADR-0015).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecallSnapshot {
    /// The execution.
    pub execution_id: ExecutionId,
    /// The owner.
    pub tenant: TenantId,
    /// When it was made.
    pub created_at: Timestamp,
    /// Ids of the memory items it holds, so deleting one can scrub it.
    pub items: Vec<String>,
    /// The content, owned by the memory crate.
    pub doc: Value,
}

/// Per-tenant memory settings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MemorySettings {
    /// The embeddings model, as `provider/model`. `None` means keyword search only.
    pub embedding_model: Option<String>,
    /// Dimensions asked of the model, when it allows choosing.
    pub embedding_dimensions: Option<u32>,
    /// A cloud provider may be used for embeddings (document text is sent to it).
    pub allow_cloud_embeddings: bool,
}

/// How a corpus is used in a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecallMode {
    /// Not used.
    Off,
    /// Retrieved at the start of the run.
    Auto,
}

/// What a run uses from memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryOptions {
    /// Earlier executions of the same conversation.
    pub conversation: bool,
    /// Long-term facts and preferences.
    pub long_term: bool,
    /// Indexed documents.
    pub knowledge: RecallMode,
    /// Past work.
    pub operations: RecallMode,
}

impl Default for MemoryOptions {
    fn default() -> Self {
        Self { conversation: true, long_term: true, knowledge: RecallMode::Auto, operations: RecallMode::Auto }
    }
}

impl MemoryOptions {
    /// Nothing is used.
    pub fn none() -> Self {
        Self { conversation: false, long_term: false, knowledge: RecallMode::Off, operations: RecallMode::Off }
    }
}

/// Where memory is kept. Every call takes the tenant; nothing crosses tenants (ADR-0013).
#[async_trait]
pub trait MemoryStore: Send + Sync {
    // ---- long-term items ----

    /// Remember something, replacing what it names. A copy of an item already in force changes nothing.
    async fn remember(&self, tenant: &TenantId, new: NewMemory, now: Timestamp) -> Result<Remembered>;
    /// One item.
    async fn memory(&self, tenant: &TenantId, id: MemoryId) -> Result<Option<MemoryItem>>;
    /// Every item of the tenant in every scope, newest first. For management (export, listing everything), never for a run.
    async fn all_memory(&self, tenant: &TenantId, filter: &MemoryFilter, now: Timestamp) -> Result<Vec<MemoryItem>>;
    /// Items visible to `visibility`, newest first.
    async fn list_memory(
        &self,
        visibility: &Visibility,
        filter: &MemoryFilter,
        now: Timestamp,
    ) -> Result<Vec<MemoryItem>>;
    /// Change an item.
    async fn update_memory(
        &self,
        tenant: &TenantId,
        id: MemoryId,
        patch: MemoryPatch,
        now: Timestamp,
    ) -> Result<MemoryItem>;
    /// Delete an item everywhere it is kept. Returns whether there was one.
    async fn forget(&self, tenant: &TenantId, id: MemoryId) -> Result<bool>;
    /// Note that items went into a context.
    async fn mark_used(&self, tenant: &TenantId, ids: &[MemoryId], now: Timestamp) -> Result<()>;

    // ---- search ----

    /// Keyword search. `terms` are already sanitised words; they are quoted and joined with OR.
    async fn text_search(
        &self,
        visibility: &Visibility,
        corpus: Corpus,
        terms: &[String],
        limit: usize,
    ) -> Result<Vec<TextHit>>;
    /// Nearest vectors of `model` (a normalised query vector).
    async fn vector_search(
        &self,
        visibility: &Visibility,
        corpus: Corpus,
        model: &str,
        query: &[f32],
        limit: usize,
    ) -> Result<Vec<VectorHit>>;

    // ---- vectors, keyed by the hash of the text ----

    /// The hashes among `hashes` that have no vector for `model`.
    async fn missing_vectors(&self, model: &str, hashes: &[String]) -> Result<Vec<String>>;
    /// Store normalised vectors.
    async fn put_vectors(&self, model: &str, vectors: Vec<(String, Vec<f32>)>) -> Result<()>;
    /// Delete the vectors of models other than `keep` and any vector no text refers to. Returns how many.
    async fn gc_vectors(&self, keep: Option<&str>) -> Result<u64>;
    /// Facts and past-work records of the tenant that have no vector for `model`, as (hash, text).
    async fn texts_without_vector(&self, tenant: &TenantId, model: &str, limit: usize)
    -> Result<Vec<(String, String)>>;
    /// How many texts have a vector for `model`, and how many texts there are.
    async fn vector_coverage(&self, tenant: &TenantId, model: &str) -> Result<(u64, u64)>;

    // ---- knowledge ----

    /// Add a folder.
    async fn add_root(&self, tenant: &TenantId, root: NewRoot, now: Timestamp) -> Result<KnowledgeRoot>;
    /// The folders visible to `visibility`.
    async fn roots(&self, visibility: &Visibility) -> Result<Vec<KnowledgeRoot>>;
    /// Every folder of a tenant.
    async fn all_roots(&self, tenant: &TenantId) -> Result<Vec<KnowledgeRoot>>;
    /// Change what a folder keeps (enabled, auto_sync, globs).
    async fn update_root(&self, tenant: &TenantId, root: KnowledgeRoot) -> Result<()>;
    /// Record the result of a sync.
    async fn finish_sync(&self, tenant: &TenantId, id: KnowledgeRootId, at: Timestamp, summary: Value) -> Result<()>;
    /// Delete a folder and everything indexed from it. The files are untouched.
    async fn remove_root(&self, tenant: &TenantId, id: KnowledgeRootId) -> Result<bool>;
    /// What is known about every file of a folder, by relative path.
    async fn document_states(&self, root: KnowledgeRootId) -> Result<HashMap<String, DocState>>;
    /// Write one file: its row and its chunks, replacing what was there.
    async fn write_document(&self, write: DocumentWrite) -> Result<()>;
    /// Forget files that are gone.
    async fn remove_documents(&self, root: KnowledgeRootId, rel_paths: &[String]) -> Result<()>;
    /// Files of a folder that failed or were skipped, with the reason.
    async fn problem_documents(&self, root: KnowledgeRootId) -> Result<Vec<(String, DocStatus, String)>>;
    /// Chunks by id.
    async fn chunks(&self, tenant: &TenantId, ids: &[i64]) -> Result<Vec<ChunkView>>;
    /// Hashes of the chunk texts of a folder that have no vector for `model`.
    async fn chunk_hashes_without_vector(
        &self,
        root: KnowledgeRootId,
        model: &str,
        limit: usize,
    ) -> Result<Vec<(String, String)>>;

    // ---- operational ----

    /// Keep the record of a finished execution (replacing an earlier one).
    async fn record_operation(&self, record: OperationRecord) -> Result<()>;
    /// One record.
    async fn operation(&self, tenant: &TenantId, id: ExecutionId) -> Result<Option<OperationRecord>>;
    /// Terminal executions that have no record yet, oldest first.
    async fn operations_missing(&self, limit: usize) -> Result<Vec<ExecutionId>>;
    /// Delete the record of an execution.
    async fn forget_operation(&self, tenant: &TenantId, id: ExecutionId) -> Result<bool>;

    // ---- snapshots and settings ----

    /// The snapshot of an execution.
    async fn snapshot(&self, id: ExecutionId) -> Result<Option<RecallSnapshot>>;
    /// Save a snapshot unless one exists; returns the one that stands.
    async fn save_snapshot(&self, snapshot: RecallSnapshot) -> Result<RecallSnapshot>;
    /// Executions whose snapshot holds this item.
    async fn snapshots_using(&self, tenant: &TenantId, item: &str) -> Result<Vec<ExecutionId>>;
    /// The settings of a tenant.
    async fn settings(&self, tenant: &TenantId) -> Result<MemorySettings>;
    /// Change the settings of a tenant.
    async fn set_settings(&self, tenant: &TenantId, settings: MemorySettings) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_and_keys_are_stable() {
        assert_eq!(bytes_hash(b"abc"), text_hash("abc"));
        assert_eq!(text_hash("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let id = MemoryId::new();
        assert_eq!(memory_key(id), format!("m:{id}"));
        assert_eq!(chunk_key(7), "k:7");
        let e = ExecutionId::new();
        assert_eq!(operation_key(e), format!("o:{e}"));
    }

    #[test]
    fn scopes_round_trip_and_visibility_is_widest_first() {
        let conversation = ConversationId::new("chat-1").unwrap();
        for scope in [Scope::Global, Scope::Project("/work/a".into()), Scope::Conversation(conversation.clone())] {
            let (name, key) = scope.parts();
            assert_eq!(Scope::from_parts(name, &key), Some(scope));
        }
        assert_eq!(Scope::from_parts("nonsense", ""), None);
        let v = Visibility {
            tenant: TenantId::local(),
            project: Some("/work/a".into()),
            conversation: Some(conversation.clone()),
        };
        assert_eq!(
            v.scopes(),
            vec![Scope::Global, Scope::Project("/work/a".into()), Scope::Conversation(conversation)]
        );
        assert!(v.sees(&Scope::Global) && !v.sees(&Scope::Project("/work/b".into())));
        assert_eq!(Visibility::tenant(TenantId::local()).scopes(), vec![Scope::Global]);
    }

    #[test]
    fn a_memory_is_checked_before_it_is_stored() {
        let ok = NewMemory {
            scope: Scope::Global,
            kind: MemoryKind::Preference,
            subject: Some("language".into()),
            content: "Answer in Portuguese.".into(),
            valid_until: None,
            origin: MemoryOrigin::User { name: "cli".into() },
            replaces: vec![],
        };
        assert!(ok.validate().is_ok());
        for bad in [
            NewMemory { content: "  ".into(), ..ok.clone() },
            NewMemory { content: "x".repeat(501), ..ok.clone() },
            NewMemory { subject: Some(" ".into()), ..ok.clone() },
            NewMemory { subject: Some("s".repeat(81)), ..ok.clone() },
        ] {
            assert!(bad.validate().is_err());
        }
    }

    #[test]
    fn an_item_is_in_force_until_it_expires_or_is_replaced() {
        let now = Timestamp(1_000);
        let mut item = MemoryItem {
            id: MemoryId::new(),
            tenant: TenantId::local(),
            scope: Scope::Global,
            kind: MemoryKind::Fact,
            subject: None,
            content: "x".into(),
            status: MemoryStatus::Active,
            superseded_by: None,
            origin: MemoryOrigin::User { name: "cli".into() },
            created_at: now,
            updated_at: now,
            confirmed_at: now,
            valid_until: Some(Timestamp(2_000)),
            last_used_at: None,
            use_count: 0,
        };
        assert!(item.in_force(now));
        assert!(!item.in_force(Timestamp(2_000)), "expired exactly at the limit");
        item.valid_until = None;
        item.status = MemoryStatus::Superseded;
        assert!(!item.in_force(now));
    }

    #[test]
    fn a_chunk_is_cited_by_lines_or_page() {
        let mut chunk = ChunkView {
            id: 1,
            root: KnowledgeRootId::new(),
            root_path: "/docs".into(),
            rel_path: "guide/a.md".into(),
            ord: 0,
            text: "t".into(),
            text_hash: "h".into(),
            heading: None,
            line_start: Some(10),
            line_end: Some(42),
            page: None,
            tokens: 5,
            indexed_at: Timestamp(0),
        };
        assert_eq!(chunk.citation(), "guide/a.md:L10-L42");
        chunk.line_start = None;
        chunk.page = Some(3);
        assert_eq!(chunk.citation(), "guide/a.md#page=3");
        chunk.page = None;
        assert_eq!(chunk.citation(), "guide/a.md");
    }

    #[test]
    fn options_default_to_using_everything_and_can_be_switched_off() {
        let all = MemoryOptions::default();
        assert!(all.conversation && all.long_term && all.knowledge == RecallMode::Auto);
        let none = MemoryOptions::none();
        assert!(
            !none.conversation
                && !none.long_term
                && none.knowledge == RecallMode::Off
                && none.operations == RecallMode::Off
        );
    }
}
