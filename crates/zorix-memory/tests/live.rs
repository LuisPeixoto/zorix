//! Memory against real embedding models and a real local model. Every test is `#[ignore]`d: run them with
//! `tools/live-tests.sh embeddings`.
//!
//! * They need a running Ollama with `all-minilm` (`ollama pull all-minilm`) and, for the last one, `qwen3:1.7b`
//!   (`ZORIX_LIVE_OLLAMA_MODEL` changes it; `ZORIX_LIVE_EMBED_MODEL` changes the embedding model).
//! * Cloud embedding tests are in the providers crate and run only with a stored key.
//!
//! Nothing here prints a whole answer or a key; only counts, ranks and short labels.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use zorix_core::{
    EmbedPurpose, EmbeddingRequest, ExecutionEngine, ExecutionState, Grants, MemoryStore, ModelGateway, NewExecution,
    Permission, Scope, TaskRequest, TenantId, Visibility,
};
use zorix_memory::MemoryStack;
use zorix_memory::index::SyncReport;
use zorix_memory::retrieve::hybrid_search;
use zorix_providers::{ProvidersConfig, Router, build_router, standard_secrets};
use zorix_store::{SqliteCalibration, SqliteMemoryStore, SqliteRepository};

fn router() -> Arc<Router> {
    let config = ProvidersConfig::default();
    let secrets = standard_secrets(&config);
    Arc::new(build_router(&config, secrets, None, false).expect("router"))
}

fn embed_model() -> String {
    format!("ollama/{}", std::env::var("ZORIX_LIVE_EMBED_MODEL").unwrap_or_else(|_| "all-minilm".into()))
}

fn docs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs")
}

async fn vectors(router: &Router, purpose: EmbedPurpose, texts: &[&str]) -> Vec<Vec<f32>> {
    let request = EmbeddingRequest {
        model: embed_model(),
        inputs: texts.iter().map(|t| t.to_string()).collect(),
        purpose,
        dimensions: None,
    };
    router
        .embed(request, CancellationToken::new())
        .await
        .unwrap_or_else(|e| panic!("`ollama pull all-minilm` first: {e}"))
        .vectors
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (norm(a) * norm(b))
}

#[tokio::test]
#[ignore = "needs a running Ollama with all-minilm"]
async fn embeddings_have_stable_dimensions_and_similar_text_scores_higher() {
    let router = router();
    let first = vectors(
        &router,
        EmbedPurpose::Document,
        &["Deploys reach staging first, then production.", "The cafeteria serves lunch at noon."],
    )
    .await;
    let again = vectors(&router, EmbedPurpose::Document, &["Deploys reach staging first, then production."]).await;
    assert_eq!(first[0].len(), first[1].len(), "every vector has the same size");
    assert_eq!(first[0].len(), again[0].len(), "and it is the same on another call");
    assert!(cosine(&first[0], &again[0]) > 0.999, "the same text gives the same vector");
    let query = vectors(&router, EmbedPurpose::Query, &["how do releases get to production?"]).await;
    let (close, far) = (cosine(&query[0], &first[0]), cosine(&query[0], &first[1]));
    eprintln!("{}: {} dimensions, similar {close:.2}, unrelated {far:.2}", embed_model(), first[0].len());
    assert!(
        close > far + 0.1,
        "a question about deploys is nearer to the deploy text ({close:.2}) than to lunch ({far:.2})"
    );
}

struct Rig {
    store: Arc<SqliteMemoryStore>,
    stack: MemoryStack,
    tenant: TenantId,
    _dir: tempfile::TempDir,
}

async fn rig(router: &Arc<Router>) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("zorix.db");
    let repository = Arc::new(SqliteRepository::open(&database).unwrap());
    let store = Arc::new(SqliteMemoryStore::open(&database).unwrap());
    let gateway: Arc<dyn ModelGateway> = router.clone();
    let stack = MemoryStack::new(store.clone(), repository, Some(gateway));
    let tenant = TenantId::local();
    store
        .set_settings(
            &tenant,
            zorix_core::MemorySettings { embedding_model: Some(embed_model()), ..Default::default() },
        )
        .await
        .unwrap();
    Rig { store, stack, tenant, _dir: dir }
}

async fn index(rig: &Rig, folder: &Path) -> SyncReport {
    let indexer = rig.stack.indexer();
    let root = indexer.add_root(&rig.tenant, Scope::Global, folder, vec![], vec![], false).await.unwrap();
    indexer.sync(&root, &|_| {}, &CancellationToken::new()).await.unwrap()
}

#[tokio::test]
#[ignore = "needs a running Ollama with all-minilm"]
async fn the_docs_of_this_repository_are_indexed_and_questions_find_the_right_file() {
    let router = router();
    let rig = rig(&router).await;
    let report = index(&rig, &docs_dir()).await;
    eprintln!("indexed {} files into {} pieces, {} vectors", report.indexed, report.chunks, report.embedded);
    assert!(report.indexed >= 10 && report.embedded >= report.chunks / 2, "{report:?}");
    assert_eq!(report.failed, 0, "{report:?}");

    // Each question is about one document, and is worded differently from it.
    let questions = [
        ("where are API keys kept for cloud providers", "providers.md"),
        ("what does a person have to approve before a tool changes a file", "permissions.md"),
        ("how do I back up the database and where is the data folder", "persistence.md"),
        ("what is the list of events a run emits", "events.md"),
        ("the app will not start because another program is using the data", "troubleshooting.md"),
    ];
    let visibility = Visibility { tenant: rig.tenant.clone(), project: None, conversation: None, agent: None };
    let embedder = rig.stack.embedder().map(|e| e.as_ref());
    let mut right = 0;
    for (question, file) in questions {
        let found =
            hybrid_search(&*rig.store, embedder, &visibility, zorix_core::Corpus::Knowledge, question).await.unwrap();
        assert!(found.degraded.is_none(), "{:?}", found.degraded);
        let ids: Vec<i64> = found.hits.iter().take(3).filter_map(|h| h.id.parse().ok()).collect();
        let chunks = rig.store.chunks(&rig.tenant, &ids).await.unwrap();
        let hit = chunks.iter().any(|c| c.rel_path.ends_with(file));
        eprintln!("{} -> {:?}", question, chunks.iter().map(|c| c.rel_path.clone()).collect::<Vec<_>>());
        right += usize::from(hit);
    }
    assert!(right >= 4, "{right} of 5 questions found their document in the top 3");

    // Nothing is relevant to this, so nothing is returned (the relevance gate).
    let unrelated = hybrid_search(
        &*rig.store,
        embedder,
        &visibility,
        zorix_core::Corpus::Knowledge,
        "recipe for sourdough bread with rye flour",
    )
    .await
    .unwrap();
    eprintln!("an unrelated question returned {} hit(s)", unrelated.hits.len());
    assert!(
        unrelated.hits.len() <= 1,
        "an unrelated question should find (almost) nothing, found {}",
        unrelated.hits.len()
    );
}

#[tokio::test]
#[ignore = "needs a running Ollama with all-minilm and a tool-capable model"]
async fn a_real_run_answers_from_an_indexed_document_and_the_context_says_so() {
    let model = std::env::var("ZORIX_LIVE_OLLAMA_MODEL").unwrap_or_else(|_| "qwen3:1.7b".into());
    let router = router();
    let rig = rig(&router).await;
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(
        folder.path().join("runbook.md"),
        "# Staging\n\nThe staging cluster is called Marmot-7. It is rebuilt every Monday at 04:00.\n\n# Production\n\nProduction is called Heron-2.\n",
    )
    .unwrap();
    std::fs::write(folder.path().join("lunch.md"), "# Lunch\n\nThe cafeteria serves lunch at noon on weekdays.\n")
        .unwrap();
    let report = index(&rig, folder.path()).await;
    assert_eq!(report.indexed, 2, "{report:?}");

    let registry = Arc::new(zorix_runtime::InMemoryToolRegistry::new());
    rig.stack.register_tools(&*registry).unwrap();
    let calibration = Arc::new(SqliteCalibration::open(&rig._dir.path().join("zorix.db")).unwrap());
    let repository = Arc::new(SqliteRepository::open(&rig._dir.path().join("zorix.db")).unwrap());
    let gateway: Arc<dyn ModelGateway> = router.clone();
    let engine = rig
        .stack
        .install(zorix_runtime::Engine::builder(repository, gateway).tools(registry).calibration(calibration.clone()))
        .build();
    let created = engine
        .create(NewExecution {
            request: TaskRequest {
                prompt:
                    "What is the staging cluster called? Answer from the documents you were given and name the file."
                        .into(),
                model: format!("ollama/{model}"),
                ..TaskRequest::default()
            },
            grants: Grants::none().with(Permission::use_tool("knowledge.search")),
            ..NewExecution::default()
        })
        .await
        .unwrap();
    let id = created.id;
    engine.start(&created.tenant, id).await.unwrap();
    let done = tokio::time::timeout(Duration::from_secs(300), engine.settle(&created.tenant, id))
        .await
        .expect("it finishes in five minutes")
        .unwrap();
    assert_eq!(done.state, ExecutionState::Completed, "{:?}", done.failure);
    let answer = done.result.expect("an answer").text;
    eprintln!("answered with {} characters; names the file: {}", answer.len(), answer.contains("runbook"));
    assert!(answer.contains("Marmot-7"), "the answer should hold the cluster name, got {} characters", answer.len());

    // What the context says it used.
    let snapshot = rig.store.snapshot(id).await.unwrap().expect("a snapshot was kept");
    let items = snapshot.doc["items"].as_array().unwrap();
    assert!(
        items.iter().any(|i| i["kind"] == "chunk" && i["label"].as_str().is_some_and(|l| l.contains("runbook.md"))),
        "{items:?}"
    );
    assert!(
        !items.iter().any(|i| i["label"].as_str().is_some_and(|l| l.contains("lunch.md"))),
        "the unrelated file stays out"
    );

    // The calibration saw the real usage (Ollama counts only the first step of a run).
    engine.shutdown().await;
    let samples = calibration.all();
    eprintln!("calibration: {:?}", samples.iter().map(|(m, c)| (m.clone(), c.samples)).collect::<Vec<_>>());
}
