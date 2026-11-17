//! Knowledge from real folders: indexing, ignoring, change detection, embedding only what is new, cancelling and resuming,
//! and finding it all again in the context and through the tool.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use zorix_core::{
    Corpus, DocStatus, EmbeddingRequest, EmbeddingResponse, ExecutionEngine, ExecutionState as S, Grants,
    KnowledgeRoot, MemoryOptions, MemorySettings, MemoryStore, ModelError, ModelGateway, ModelRequest, ModelResponse,
    NewExecution, Permission, Scope, TaskRequest, TenantId, Visibility,
};
use zorix_memory::index::{Indexer, Phase, Progress, SyncReport};
use zorix_memory::{Embedder, MemoryStack};
use zorix_runtime::{Engine, EngineConfig, InMemoryToolRegistry};
use zorix_sim::{DemoModel, ScriptedModel, Turn};
use zorix_store::{SqliteMemoryStore, SqliteRepository};

struct Env {
    _dir: tempfile::TempDir,
    folder: std::path::PathBuf,
    repository: Arc<SqliteRepository>,
    store: Arc<SqliteMemoryStore>,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zorix.sqlite");
    let folder = dir.path().join("docs");
    std::fs::create_dir_all(&folder).unwrap();
    Env {
        repository: Arc::new(SqliteRepository::open(&path).unwrap()),
        store: Arc::new(SqliteMemoryStore::open(&path).unwrap()),
        folder,
        _dir: dir,
    }
}

fn write(folder: &Path, rel: &str, content: &str) {
    let path = folder.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

/// A change the file system will notice even on coarse clocks.
fn rewrite(folder: &Path, rel: &str, content: &str) {
    let path = folder.join(rel);
    let before = std::fs::metadata(&path).unwrap().modified().unwrap();
    std::fs::write(&path, content).unwrap();
    let later = before + std::time::Duration::from_secs(5);
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_modified(later).unwrap();
}

fn tenant() -> TenantId {
    TenantId::local()
}

fn vis() -> Visibility {
    Visibility::tenant(tenant())
}

async fn add(env: &Env, indexer: &Indexer) -> KnowledgeRoot {
    indexer.add_root(&tenant(), Scope::Global, &env.folder, vec![], vec![], false).await.unwrap()
}

async fn sync(indexer: &Indexer, root: &KnowledgeRoot) -> (SyncReport, Vec<Progress>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let report = indexer.sync(root, &move |p| sink.lock().push(p), &CancellationToken::new()).await.unwrap();
    let progress = seen.lock().clone();
    (report, progress)
}

async fn count_hits(env: &Env, word: &str) -> usize {
    env.store.text_search(&vis(), Corpus::Knowledge, &[word.to_string()], 20).await.unwrap().len()
}

fn fill_folder(folder: &Path) {
    write(
        folder,
        "README.md",
        "# Handbook\nWelcome to the handbook of the platform team.\n\n## Deploys\nDeploys reach staging first, then production.\n",
    );
    write(folder, "src/deploy.rs", "fn deploy() {\n    // push to staging\n}\n");
    write(
        folder,
        "web/index.html",
        "<html><body><h1>Status</h1><p>The dashboard shows canary health.</p><script>var leak = 'hidden-script';</script></body></html>",
    );
    write(folder, "notes.txt", "Remember the quarterly review of the billing service.");
    write(folder, ".gitignore", "drafts/\n*.log\n");
    write(folder, "drafts/secret-plan.md", "Draft: acquisition of a competitor.");
    write(folder, "build.log", "noise noise noise");
    write(folder, ".env", "API_KEY=sk-abcdefghijklmnopqrstuvwxyz0123456789");
    write(folder, "config/secrets.yaml", "token: shh");
    write(folder, "image.png", "\u{0}PNG binary");
    std::fs::write(folder.join("blob.txt"), [0u8, 1, 2, 3, 0, 5]).unwrap();
}

#[tokio::test]
async fn a_folder_is_indexed_with_what_belongs_in_it_and_nothing_private() {
    let env = env();
    fill_folder(&env.folder);
    let indexer = Indexer::new(env.store.clone(), None);
    let root = add(&env, &indexer).await;
    assert_eq!(root.path, std::fs::canonicalize(&env.folder).unwrap().to_string_lossy());
    let (report, progress) = sync(&indexer, &root).await;
    assert_eq!((report.indexed, report.unchanged, report.removed), (4, 0, 0), "{report:?}");
    assert_eq!(report.skipped, 1, "the binary text file is left out with a reason");
    assert_eq!(report.failed, 0);
    assert!(report.chunks >= 4);

    // What is in the folder can be found, with its source.
    assert_eq!(count_hits(&env, "staging").await, 2, "the Markdown section and the Rust file");
    assert_eq!(count_hits(&env, "canary").await, 1, "HTML text is read");
    assert_eq!(count_hits(&env, "quarterly").await, 1);
    // What must not be there is not.
    for hidden in ["acquisition", "noise", "sk-abcdefghijklmnopqrstuvwxyz0123456789", "shh", "hidden-script", "API_KEY"]
    {
        assert_eq!(count_hits(&env, hidden).await, 0, "{hidden}");
    }
    let states = env.store.document_states(root.id).await.unwrap();
    let mut files: Vec<&str> = states.keys().map(String::as_str).collect();
    files.sort_unstable();
    assert_eq!(files, ["README.md", "blob.txt", "notes.txt", "src/deploy.rs", "web/index.html"]);
    assert_eq!(
        env.store.problem_documents(root.id).await.unwrap(),
        vec![("blob.txt".to_string(), DocStatus::Skipped, "it is a binary file".to_string())]
    );

    // Progress goes scanning, indexing, done, and the report is stored on the folder.
    let phases: Vec<Phase> = progress.iter().map(|p| p.phase).collect();
    assert_eq!(phases.first(), Some(&Phase::Scanning));
    assert_eq!(phases.last(), Some(&Phase::Done));
    assert!(
        phases.contains(&Phase::Indexing) && !phases.contains(&Phase::Embedding),
        "no embeddings model, no embedding phase"
    );
    let stored = env.store.all_roots(&tenant()).await.unwrap().remove(0);
    assert_eq!(stored.last_sync.as_ref().unwrap()["indexed"], json!(4));
    assert!(stored.last_sync_at.is_some());
}

#[tokio::test]
async fn a_second_sync_does_nothing_until_a_file_changes_and_then_only_that_file() {
    let env = env();
    fill_folder(&env.folder);
    let indexer = Indexer::new(env.store.clone(), None);
    let root = add(&env, &indexer).await;
    sync(&indexer, &root).await;
    let (again, _) = sync(&indexer, &root).await;
    assert_eq!((again.indexed, again.unchanged, again.removed), (0, 5, 0), "{again:?}");

    rewrite(&env.folder, "notes.txt", "The review moved to the retrospective of the archive team.");
    let (changed, _) = sync(&indexer, &root).await;
    assert_eq!((changed.indexed, changed.unchanged), (1, 4), "{changed:?}");
    assert_eq!(count_hits(&env, "quarterly").await, 0, "the old text is gone");
    assert_eq!(count_hits(&env, "retrospective").await, 1);

    // A file deleted from the folder, and a file that is now ignored, leave the index.
    std::fs::remove_file(env.folder.join("src/deploy.rs")).unwrap();
    write(&env.folder, ".gitignore", "drafts/\n*.log\nweb/\n");
    let (gone, _) = sync(&indexer, &root).await;
    assert_eq!(gone.removed, 2, "{gone:?}");
    assert_eq!(count_hits(&env, "canary").await, 0);
    assert_eq!(count_hits(&env, "staging").await, 1, "only the Markdown is left");
}

#[tokio::test]
async fn a_folder_that_disappears_is_reported_not_emptied() {
    let env = env();
    write(&env.folder, "a.md", "alpha text");
    let indexer = Indexer::new(env.store.clone(), None);
    let root = add(&env, &indexer).await;
    sync(&indexer, &root).await;
    std::fs::remove_dir_all(&env.folder).unwrap();
    let error = indexer.sync(&root, &|_| {}, &CancellationToken::new()).await.unwrap_err();
    assert!(error.to_string().contains("not there any more"), "{error}");
    assert_eq!(count_hits(&env, "alpha").await, 1, "what was indexed stays until the folder is removed on purpose");
}

#[tokio::test]
async fn adding_a_folder_checks_the_path_and_the_patterns() {
    let env = env();
    let indexer = Indexer::new(env.store.clone(), None);
    let missing =
        indexer.add_root(&tenant(), Scope::Global, &env.folder.join("nope"), vec![], vec![], false).await.unwrap_err();
    assert!(missing.to_string().contains("cannot be used"), "{missing}");
    write(&env.folder, "file.md", "x");
    let not_a_folder = indexer
        .add_root(&tenant(), Scope::Global, &env.folder.join("file.md"), vec![], vec![], false)
        .await
        .unwrap_err();
    assert!(not_a_folder.to_string().contains("not a folder"));
    let bad =
        indexer.add_root(&tenant(), Scope::Global, &env.folder, vec!["[".into()], vec![], false).await.unwrap_err();
    assert!(bad.to_string().contains("not a valid pattern"), "{bad}");
    add(&env, &indexer).await;
    assert!(
        indexer.add_root(&tenant(), Scope::Global, &env.folder, vec![], vec![], false).await.is_err(),
        "the same folder twice"
    );
}

#[tokio::test]
async fn one_file_that_cannot_be_read_does_not_stop_the_others() {
    let env = env();
    write(&env.folder, "good.md", "# Good\nreadable text about invoices");
    std::fs::write(
        env.folder.join("latin.txt"),
        "ação é coração ".repeat(30).chars().map(|c| c as u32 as u8).collect::<Vec<u8>>(),
    )
    .unwrap();
    let indexer = Indexer::new(env.store.clone(), None);
    let root = add(&env, &indexer).await;
    let (report, _) = sync(&indexer, &root).await;
    assert_eq!((report.indexed, report.skipped, report.failed), (1, 1, 0), "{report:?}");
    assert_eq!(count_hits(&env, "invoices").await, 1);
    assert_eq!(env.store.problem_documents(root.id).await.unwrap()[0].2, "it is not UTF-8 text");
}

// ---------------------------------------------------------------------------------------------------------------------
// vectors

struct Counting {
    inner: DemoModel,
    texts: AtomicUsize,
    calls: AtomicUsize,
    broken: bool,
}

impl Counting {
    fn new(broken: bool) -> Arc<Self> {
        Arc::new(Self { inner: DemoModel::new(), texts: AtomicUsize::new(0), calls: AtomicUsize::new(0), broken })
    }
}

#[async_trait]
impl ModelGateway for Counting {
    async fn generate(&self, request: ModelRequest, cancel: CancellationToken) -> Result<ModelResponse, ModelError> {
        self.inner.generate(request, cancel).await
    }

    async fn embed(
        &self,
        request: EmbeddingRequest,
        cancel: CancellationToken,
    ) -> Result<EmbeddingResponse, ModelError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.broken {
            return Err(ModelError::provider(zorix_core::ModelErrorKind::Unavailable, "the embeddings server is down"));
        }
        self.texts.fetch_add(request.inputs.len(), Ordering::SeqCst);
        self.inner.embed(request, cancel).await
    }
}

async fn with_embeddings(env: &Env, gateway: Arc<Counting>) -> Indexer {
    env.store
        .set_settings(
            &tenant(),
            MemorySettings {
                embedding_model: Some("sim/hash-embed".into()),
                embedding_dimensions: None,
                allow_cloud_embeddings: false,
            },
        )
        .await
        .unwrap();
    Indexer::new(env.store.clone(), Some(Arc::new(Embedder::new(gateway))))
}

#[tokio::test]
async fn only_new_text_is_embedded_and_unchanged_files_cost_nothing() {
    let env = env();
    write(&env.folder, "a.md", "# A\nAlpha text about deploys.");
    write(&env.folder, "b.md", "# B\nBeta text about rollbacks.");
    let gateway = Counting::new(false);
    let indexer = with_embeddings(&env, gateway.clone()).await;
    let root = add(&env, &indexer).await;
    let (first, progress) = sync(&indexer, &root).await;
    assert_eq!((first.indexed, first.embedded), (2, 2), "{first:?}");
    assert_eq!(gateway.texts.load(Ordering::SeqCst), 2);
    assert!(progress.iter().any(|p| p.phase == Phase::Embedding && p.done == 2 && p.total == 2));
    assert_eq!(env.store.vector_coverage(&tenant(), "sim/hash-embed").await.unwrap(), (2, 2));

    // Nothing changed: nothing is embedded.
    let before = gateway.calls.load(Ordering::SeqCst);
    let (second, _) = sync(&indexer, &root).await;
    assert_eq!((second.indexed, second.embedded), (0, 0));
    assert_eq!(gateway.calls.load(Ordering::SeqCst), before, "no call at all");

    // One file changes: only its new text is embedded, and moving text elsewhere costs nothing.
    rewrite(&env.folder, "a.md", "# A\nAlpha text about canary releases.");
    write(&env.folder, "copy-of-b.md", "# B\nBeta text about rollbacks.");
    let (third, _) = sync(&indexer, &root).await;
    assert_eq!(third.indexed, 2, "{third:?}");
    assert_eq!(third.embedded, 1, "the new alpha text; the copy of beta shares its vector");
    assert_eq!(
        env.store.vector_coverage(&tenant(), "sim/hash-embed").await.unwrap(),
        (2, 2),
        "two distinct texts now: the new alpha and beta, which both files share"
    );
}

#[tokio::test]
async fn when_embeddings_fail_the_text_is_still_searchable_and_the_report_says_why() {
    let env = env();
    write(&env.folder, "a.md", "# A\nAlpha text about deploys.");
    let indexer = with_embeddings(&env, Counting::new(true)).await;
    let root = add(&env, &indexer).await;
    let (report, _) = sync(&indexer, &root).await;
    assert_eq!((report.indexed, report.embedded), (1, 0));
    assert!(
        report.degraded.as_deref().is_some_and(|d| d.contains("down") && d.contains("keyword search still works")),
        "{:?}",
        report.degraded
    );
    assert_eq!(count_hits(&env, "deploys").await, 1);
    // Once the server is back, the next sync embeds what is missing.
    let recovered = with_embeddings(&env, Counting::new(false)).await;
    let (later, _) = sync(&recovered, &root).await;
    assert_eq!((later.indexed, later.embedded), (0, 1));
}

#[tokio::test]
async fn cancelling_keeps_what_is_done_and_the_next_sync_finishes_the_job() {
    let env = env();
    for n in 0..6 {
        write(&env.folder, &format!("f{n}.md"), &format!("# F{n}\nText number {n} about topic{n}."));
    }
    let indexer = Indexer::new(env.store.clone(), None);
    let root = add(&env, &indexer).await;
    let cancel = CancellationToken::new();
    let stop = cancel.clone();
    let files = Arc::new(AtomicUsize::new(0));
    let counter = files.clone();
    let report = indexer
        .sync(
            &root,
            &move |p: Progress| {
                if p.phase == Phase::Indexing && p.file.is_some() && counter.fetch_add(1, Ordering::SeqCst) == 2 {
                    stop.cancel();
                }
            },
            &cancel,
        )
        .await
        .unwrap();
    assert!(report.cancelled);
    assert!(report.indexed >= 2 && report.indexed < 6, "{report:?}");
    let partial = env.store.document_states(root.id).await.unwrap().len();
    assert_eq!(partial as u64, report.indexed);
    assert!(env.store.all_roots(&tenant()).await.unwrap()[0].last_sync.as_ref().unwrap()["cancelled"] == json!(true));
    let (rest, _) = sync(&indexer, &root).await;
    assert_eq!((rest.indexed, rest.unchanged), (6 - report.indexed, report.indexed));
    assert!(!rest.cancelled);
}

// ---------------------------------------------------------------------------------------------------------------------
// in the context and through the tool

fn grants() -> Grants {
    Grants::none().with(Permission::use_tool("knowledge.search"))
}

#[tokio::test]
async fn an_indexed_folder_reaches_the_context_and_the_search_tool_with_citations() {
    let env = env();
    fill_folder(&env.folder);
    let stack = MemoryStack::new(env.store.clone(), env.repository.clone(), None);
    let indexer = stack.indexer();
    let root = add(&env, &indexer).await;
    sync(&indexer, &root).await;
    let registry = Arc::new(InMemoryToolRegistry::new());
    stack.register_tools(&*registry).unwrap();
    let model = Arc::new(ScriptedModel::new([
        Turn::Say("Staging first.".into()),
        Turn::call("knowledge.search", json!({"query": "canary dashboard health"})),
        Turn::Say("The dashboard shows canary health.".into()),
    ]));
    let engine = stack
        .install(Engine::builder(env.repository.clone(), model.clone()).tools(registry))
        .config(EngineConfig::default())
        .build();
    let ask = |prompt: &str| NewExecution {
        request: TaskRequest { prompt: prompt.into(), model: "sim".into(), ..TaskRequest::default() },
        grants: grants(),
        memory: MemoryOptions::default(),
        ..NewExecution::default()
    };
    let first = engine.submit(ask("How do deploys reach staging and then production?")).await.unwrap();
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(10), engine.settle(&tenant(), first.id))
            .await
            .unwrap()
            .unwrap()
            .state,
        S::Completed
    );
    let documents: Vec<String> = model.requests()[0]
        .messages
        .iter()
        .filter(|m| m.content.starts_with("<document"))
        .map(|m| m.content.clone())
        .collect();
    assert!(
        documents.iter().any(|d| d.contains("source=\"README.md:") && d.contains("Deploys reach staging first")),
        "{documents:?}"
    );
    assert!(!documents.iter().any(|d| d.contains("acquisition") || d.contains("shh")));

    let second = engine.submit(ask("What does the status dashboard show?")).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), engine.settle(&tenant(), second.id))
        .await
        .unwrap()
        .unwrap();
    let result = model.requests()[2]
        .messages
        .iter()
        .find(|m| m.content.starts_with("<tool_result"))
        .expect("the tool result")
        .content
        .clone();
    assert!(result.contains("web/index.html") && result.contains("canary health"), "{result}");
}

#[tokio::test]
async fn removing_a_folder_removes_its_index_and_leaves_the_files() {
    let env = env();
    write(&env.folder, "a.md", "# A\nA unique zebracorn passage.");
    let indexer = Indexer::new(env.store.clone(), None);
    let root = add(&env, &indexer).await;
    sync(&indexer, &root).await;
    assert_eq!(count_hits(&env, "zebracorn").await, 1);
    assert!(env.store.remove_root(&tenant(), root.id).await.unwrap());
    assert_eq!(count_hits(&env, "zebracorn").await, 0);
    assert!(env.folder.join("a.md").exists(), "the files are untouched");
}

#[cfg(unix)]
#[tokio::test]
async fn a_link_that_points_outside_the_folder_is_never_followed() {
    let env = env();
    let outside = env._dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("private.md"), "Outside text with a zebracorn secret.").unwrap();
    write(&env.folder, "inside.md", "Inside text.");
    std::os::unix::fs::symlink(&outside, env.folder.join("link")).unwrap();
    std::os::unix::fs::symlink(outside.join("private.md"), env.folder.join("file-link.md")).unwrap();
    let indexer = Indexer::new(env.store.clone(), None);
    let root = add(&env, &indexer).await;
    let (report, _) = sync(&indexer, &root).await;
    assert_eq!(report.indexed, 1, "{report:?}");
    assert_eq!(count_hits(&env, "zebracorn").await, 0);
}
