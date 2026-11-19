//! `notes.put` and `notes.get`: a tiny key-value notebook, kept per tenant in one JSON file.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::{Value, json};
use zorix_core::{Tool, ToolAnnotations, ToolContext, ToolError, ToolSpec};

/// Longest key.
pub const MAX_KEY: usize = 64;
/// Longest value, in bytes.
pub const MAX_VALUE: usize = 4 * 1024;
/// Most notes per tenant.
pub const MAX_NOTES: usize = 1_000;

type Book = BTreeMap<String, BTreeMap<String, String>>;

/// The notebook behind both tools. One file; every change is written whole through a temporary file and a rename.
pub struct Notebook {
    path: PathBuf,
    lock: Mutex<()>,
}

fn failed(message: impl Into<String>) -> ToolError {
    ToolError::Failed(message.into())
}

fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_KEY
        && key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

impl Notebook {
    /// A notebook stored in the file at `path` (created on the first note).
    pub fn new(path: PathBuf) -> Arc<Self> {
        Arc::new(Self { path, lock: Mutex::new(()) })
    }

    fn load(&self) -> Result<Book, ToolError> {
        match fs::read_to_string(&self.path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| failed(format!("The notes file is damaged: {e}"))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Book::new()),
            Err(e) => Err(failed(format!("Cannot read the notes file: {e}"))),
        }
    }

    fn store(&self, book: &Book) -> Result<(), ToolError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| failed(format!("Cannot create the notes folder: {e}")))?;
        }
        let temporary = self.path.with_extension("tmp");
        let text = serde_json::to_string_pretty(book).map_err(|e| failed(e.to_string()))?;
        fs::write(&temporary, text).map_err(|e| failed(format!("Cannot write the notes: {e}")))?;
        fs::rename(&temporary, &self.path).map_err(|e| failed(format!("Cannot write the notes: {e}")))
    }

    /// Save a note for a tenant.
    pub fn put(&self, tenant: &str, key: &str, value: &str) -> Result<(), ToolError> {
        if !valid_key(key) {
            return Err(failed(format!("A note name has 1 to {MAX_KEY} letters, digits, '-', '_' or '.'.")));
        }
        if value.len() > MAX_VALUE {
            return Err(failed(format!("A note holds at most {MAX_VALUE} bytes.")));
        }
        let _guard = self.lock.lock();
        let mut book = self.load()?;
        let notes = book.entry(tenant.to_string()).or_default();
        if !notes.contains_key(key) && notes.len() >= MAX_NOTES {
            return Err(failed(format!("There are already {MAX_NOTES} notes.")));
        }
        notes.insert(key.to_string(), value.to_string());
        self.store(&book)
    }

    /// Read a note of a tenant.
    pub fn get(&self, tenant: &str, key: &str) -> Result<Option<String>, ToolError> {
        let _guard = self.lock.lock();
        Ok(self.load()?.get(tenant).and_then(|notes| notes.get(key)).cloned())
    }
}

/// The `notes.put` tool. Every call asks a person first.
pub struct PutNoteTool {
    spec: ToolSpec,
    notebook: Arc<Notebook>,
}

/// The `notes.get` tool.
pub struct GetNoteTool {
    spec: ToolSpec,
    notebook: Arc<Notebook>,
}

impl PutNoteTool {
    /// A tool that writes to `notebook`.
    pub fn new(notebook: Arc<Notebook>) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {
                "key": {"type": "string", "maxLength": MAX_KEY},
                "value": {"type": "string", "maxLength": MAX_VALUE}
            },
            "required": ["key", "value"],
            "additionalProperties": false
        });
        let mut spec = ToolSpec::new(
            "notes.put",
            "Save note",
            "Saves a short note under a name, replacing any earlier note with that name.",
            schema,
        );
        spec.annotations =
            ToolAnnotations { read_only: false, destructive: false, idempotent: true, open_world: false };
        spec.requires_approval = true;
        Self { spec, notebook }
    }
}

impl GetNoteTool {
    /// A tool that reads from `notebook`.
    pub fn new(notebook: Arc<Notebook>) -> Self {
        let schema = json!({
            "type": "object",
            "properties": {"key": {"type": "string", "maxLength": MAX_KEY}},
            "required": ["key"],
            "additionalProperties": false
        });
        let mut spec = ToolSpec::new("notes.get", "Read note", "Reads the note saved under a name.", schema);
        spec.annotations = ToolAnnotations::safe_read();
        Self { spec, notebook }
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, ToolError> + Send + 'static,
) -> Result<T, ToolError> {
    tokio::task::spawn_blocking(work).await.map_err(|e| failed(format!("The notes task failed: {e}")))?
}

#[async_trait]
impl Tool for PutNoteTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let notebook = self.notebook.clone();
        let tenant = ctx.tenant.as_str().to_string();
        let key = args["key"].as_str().unwrap_or_default().to_string();
        let value = args["value"].as_str().unwrap_or_default().to_string();
        blocking(move || notebook.put(&tenant, &key, &value).map(|()| json!({ "saved": key }))).await
    }
}

#[async_trait]
impl Tool for GetNoteTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let notebook = self.notebook.clone();
        let tenant = ctx.tenant.as_str().to_string();
        let key = args["key"].as_str().unwrap_or_default().to_string();
        blocking(move || {
            notebook
                .get(&tenant, &key)?
                .map(|value| json!({ "key": key, "value": value }))
                .ok_or_else(|| failed(format!("There is no note called {key}.")))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use zorix_core::{ExecutionId, Grants, TenantId, ToolCallId};

    use super::*;

    fn context(tenant: &str) -> ToolContext {
        ToolContext {
            execution_id: ExecutionId::new(),
            tenant: TenantId::new(tenant).unwrap(),
            call_id: ToolCallId("c".into()),
            grants: Grants::none(),
            cancel: tokio_util::sync::CancellationToken::new(),
            workspace_root: None,
            conversation: None,
            agent: None,
        }
    }

    #[tokio::test]
    async fn notes_are_saved_read_and_survive_a_new_notebook_on_the_same_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/notes.json");
        let put = PutNoteTool::new(Notebook::new(path.clone()));
        assert_eq!(
            put.call(&context("acme"), json!({"key": "result", "value": "17*23 = 391"})).await.unwrap(),
            json!({"saved": "result"})
        );
        let get = GetNoteTool::new(Notebook::new(path));
        let found = get.call(&context("acme"), json!({"key": "result"})).await.unwrap();
        assert_eq!(found, json!({"key": "result", "value": "17*23 = 391"}));
    }

    #[tokio::test]
    async fn one_tenant_never_sees_another_tenants_notes() {
        let dir = tempfile::tempdir().unwrap();
        let book = Notebook::new(dir.path().join("notes.json"));
        PutNoteTool::new(book.clone()).call(&context("acme"), json!({"key": "k", "value": "acme's"})).await.unwrap();
        let get = GetNoteTool::new(book);
        assert!(get.call(&context("globex"), json!({"key": "k"})).await.is_err());
        assert_eq!(get.call(&context("acme"), json!({"key": "k"})).await.unwrap()["value"], "acme's");
    }

    #[tokio::test]
    async fn a_missing_note_is_an_error_the_model_can_read() {
        let dir = tempfile::tempdir().unwrap();
        let get = GetNoteTool::new(Notebook::new(dir.path().join("notes.json")));
        let error = get.call(&context("acme"), json!({"key": "nope"})).await.unwrap_err();
        assert_eq!(error, ToolError::Failed("There is no note called nope.".into()));
    }

    #[test]
    fn names_and_sizes_are_limited() {
        let dir = tempfile::tempdir().unwrap();
        let book = Notebook::new(dir.path().join("notes.json"));
        for bad in ["", "a b", "a/b", "../x", &"k".repeat(MAX_KEY + 1)] {
            assert!(book.put("acme", bad, "v").is_err(), "{bad:?}");
        }
        assert!(book.put("acme", "ok", &"v".repeat(MAX_VALUE + 1)).is_err());
        assert!(book.put("acme", "ok.name-1_2", "v").is_ok());
        for n in 0..MAX_NOTES - 1 {
            book.put("acme", &format!("n{n}"), "v").unwrap();
        }
        assert!(book.put("acme", "one-too-many", "v").is_err());
        assert!(book.put("acme", "n1", "replacing is still fine").is_ok());
    }

    #[test]
    fn a_damaged_file_is_reported_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.json");
        fs::write(&path, "{broken").unwrap();
        let book = Notebook::new(path.clone());
        assert!(book.put("acme", "k", "v").is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "{broken");
    }

    #[test]
    fn saving_asks_for_approval_and_reading_does_not() {
        let book = Notebook::new(PathBuf::from("unused.json"));
        assert!(PutNoteTool::new(book.clone()).spec().requires_approval);
        assert!(!GetNoteTool::new(book).spec().requires_approval);
    }
}
