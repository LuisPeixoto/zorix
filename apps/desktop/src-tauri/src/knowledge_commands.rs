//! The commands behind the Knowledge and History tabs and the embeddings card. A sync runs in the background and reports
//! with `knowledge://progress` and `knowledge://done`; closing the app cancels it and what is done stays indexed.

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, State};
use tokio_util::sync::CancellationToken;
use zorix_core::{
    Corpus, DocStatus, EmbedPurpose, EmbeddingRequest, ExecutionId, KnowledgeRoot, KnowledgeRootId, MemorySettings,
    MemoryStore, ModelGateway, Scope, Visibility,
};
use zorix_memory::retrieve::hybrid_search;
use zorix_providers::embedding_choices;

use crate::state::{AppState, Services};
use crate::ui_error::{UiError, ui};

fn parse_root(text: &str) -> Result<KnowledgeRootId, UiError> {
    text.parse().map_err(|_| UiError::message(format!("“{text}” is not a folder id.")))
}

/// A folder as the screen shows it.
#[derive(Serialize)]
pub struct RootRow {
    id: String,
    path: String,
    scope: String,
    enabled: bool,
    auto_sync: bool,
    include: Vec<String>,
    exclude: Vec<String>,
    last_sync_at: Option<i64>,
    last_sync: Option<Value>,
    indexed: u64,
    skipped: u64,
    failed: u64,
    syncing: bool,
}

async fn row(services: &Services, root: &KnowledgeRoot) -> Result<RootRow, UiError> {
    let states = services.memory_store.document_states(root.id).await.map_err(ui)?;
    let count = |wanted: DocStatus| states.values().filter(|s| s.status == wanted).count() as u64;
    Ok(RootRow {
        id: root.id.to_string(),
        path: root.path.clone(),
        scope: root.scope.label(),
        enabled: root.enabled,
        auto_sync: root.auto_sync,
        include: root.include.clone(),
        exclude: root.exclude.clone(),
        last_sync_at: root.last_sync_at.map(|t| t.millis()),
        last_sync: root.last_sync.clone(),
        indexed: count(DocStatus::Indexed),
        skipped: count(DocStatus::Skipped),
        failed: count(DocStatus::Failed),
        syncing: services.syncs.lock().contains_key(&root.id),
    })
}

#[tauri::command]
pub async fn list_roots(state: State<'_, AppState>) -> Result<Vec<RootRow>, UiError> {
    let services = state.services()?;
    let roots = services.memory_store.all_roots(&services.tenant).await.map_err(ui)?;
    let mut rows = Vec::new();
    for root in &roots {
        rows.push(row(services, root).await?);
    }
    Ok(rows)
}

/// Start a sync in the background, unless one is already running for this folder.
fn spawn_sync(app: AppHandle, services: &Arc<Services>, root: KnowledgeRoot) -> bool {
    let token = CancellationToken::new();
    {
        let mut running = services.syncs.lock();
        if running.contains_key(&root.id) {
            return false;
        }
        running.insert(root.id, token.clone());
    }
    let services = services.clone();
    tokio::spawn(async move {
        let indexer = services.memory.indexer();
        let id = root.id.to_string();
        let sink = app.clone();
        let progress_id = id.clone();
        let progress = move |p: zorix_memory::index::Progress| {
            let _ = sink.emit("knowledge://progress", json!({ "root": progress_id, "progress": p }));
        };
        let outcome = indexer.sync(&root, &progress, &token).await;
        services.syncs.lock().remove(&root.id);
        let payload = match outcome {
            Ok(report) => json!({ "root": id, "report": report, "error": null }),
            Err(error) => json!({ "root": id, "report": null, "error": error.to_string() }),
        };
        let _ = app.emit("knowledge://done", payload);
    });
    true
}

/// Bring the folders marked "check at start" up to date. A cheap pass: unchanged files are only looked at, not read.
pub async fn sync_at_start(app: AppHandle) {
    let Some(state) = tauri::Manager::try_state::<AppState>(&app) else { return };
    let Ok(services) = state.services_arc() else { return };
    let Ok(roots) = services.memory_store.all_roots(&services.tenant).await else { return };
    for root in roots.into_iter().filter(|r| r.enabled && r.auto_sync) {
        spawn_sync(app.clone(), &services, root);
    }
}

#[derive(Deserialize)]
pub struct NewRoot {
    path: String,
    /// A project folder, to keep this knowledge to runs in that project. Left out: for every run.
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    include: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(default)]
    auto_sync: bool,
}

#[tauri::command]
pub async fn add_root(app: AppHandle, state: State<'_, AppState>, root: NewRoot) -> Result<RootRow, UiError> {
    let services = state.services_arc()?;
    let scope = match root.project.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(dir) => Scope::Project(
            std::fs::canonicalize(dir)
                .map_err(|e| UiError::message(format!("The project folder {dir} cannot be used: {e}")))?
                .to_string_lossy()
                .into_owned(),
        ),
        None => Scope::Global,
    };
    let added = services
        .memory
        .indexer()
        .add_root(&services.tenant, scope, &PathBuf::from(root.path.trim()), root.include, root.exclude, root.auto_sync)
        .await
        .map_err(ui)?;
    spawn_sync(app, &services, added.clone());
    row(&services, &added).await
}

#[tauri::command]
pub async fn sync_root(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<bool, UiError> {
    let services = state.services_arc()?;
    let id = parse_root(&id)?;
    let root = services
        .memory_store
        .all_roots(&services.tenant)
        .await
        .map_err(ui)?
        .into_iter()
        .find(|r| r.id == id)
        .ok_or("That folder is not in the index.")?;
    Ok(spawn_sync(app, &services, root))
}

/// Stop a sync between files. What is done stays indexed.
#[tauri::command]
pub async fn cancel_sync(state: State<'_, AppState>, id: String) -> Result<(), UiError> {
    let services = state.services()?;
    if let Some(token) = services.syncs.lock().get(&parse_root(&id)?) {
        token.cancel();
    }
    Ok(())
}

#[tauri::command]
pub async fn remove_root(state: State<'_, AppState>, id: String) -> Result<bool, UiError> {
    let services = state.services()?;
    let id = parse_root(&id)?;
    if let Some(token) = services.syncs.lock().get(&id) {
        token.cancel();
    }
    let removed = services.memory_store.remove_root(&services.tenant, id).await.map_err(ui)?;
    services.memory.recall().invalidate_all();
    Ok(removed)
}

#[derive(Serialize)]
pub struct FileRow {
    file: String,
    status: String,
    reason: Option<String>,
}

/// The files of a folder, or only the ones that failed or were left out, with the reason.
#[tauri::command]
pub async fn root_files(state: State<'_, AppState>, id: String, problems_only: bool) -> Result<Vec<FileRow>, UiError> {
    let services = state.services()?;
    let id = parse_root(&id)?;
    if problems_only {
        let problems = services.memory_store.problem_documents(id).await.map_err(ui)?;
        return Ok(problems
            .into_iter()
            .map(|(file, status, why)| FileRow { file, status: status.as_str().into(), reason: Some(why) })
            .collect());
    }
    let mut files: Vec<FileRow> = services
        .memory_store
        .document_states(id)
        .await
        .map_err(ui)?
        .into_iter()
        .map(|(file, s)| FileRow { file, status: s.status.as_str().into(), reason: None })
        .collect();
    files.sort_by(|a, b| a.file.cmp(&b.file));
    files.truncate(2_000);
    Ok(files)
}

/// One result of a try-it search.
#[derive(Serialize)]
pub struct Hit {
    title: String,
    text: String,
    keyword_rank: Option<u32>,
    vector_rank: Option<u32>,
    cosine: Option<f32>,
    execution: Option<String>,
    state: Option<String>,
}

#[derive(Serialize)]
pub struct Found {
    hits: Vec<Hit>,
    degraded: Option<String>,
}

fn cut(text: &str, max: usize) -> String {
    zorix_memory::text::cut(&text.replace('\n', " "), max)
}

/// Search the documents or past work the way a run would, and say which list found each hit.
#[tauri::command]
pub async fn try_query(
    state: State<'_, AppState>,
    query: String,
    corpus: String,
    project: Option<String>,
) -> Result<Found, UiError> {
    let services = state.services()?;
    let project = match project.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(dir) => Some(
            std::fs::canonicalize(dir)
                .map_err(|e| UiError::message(format!("The project folder {dir} cannot be used: {e}")))?
                .to_string_lossy()
                .into_owned(),
        ),
        None => None,
    };
    let visibility = Visibility { tenant: services.tenant.clone(), project, conversation: None, agent: None };
    let (which, store) = match corpus.as_str() {
        "knowledge" => (Corpus::Knowledge, &services.memory_store),
        "history" => (Corpus::Operations, &services.memory_store),
        _ => return Err("Search the documents or the history.".into()),
    };
    let found = hybrid_search(&**store, services.memory.embedder().map(|e| e.as_ref()), &visibility, which, &query)
        .await
        .map_err(ui)?;
    let mut hits = Vec::new();
    for hit in found.hits.iter().take(10) {
        let (title, text, execution, state) = match which {
            Corpus::Knowledge => {
                let chunk = match hit.id.parse::<i64>() {
                    Ok(id) => {
                        services.memory_store.chunks(&services.tenant, &[id]).await.map_err(ui)?.into_iter().next()
                    }
                    Err(_) => None,
                };
                match chunk {
                    Some(c) => (c.citation(), cut(&c.text, 260), None, None),
                    None => continue,
                }
            }
            _ => {
                let record = match hit.id.parse::<ExecutionId>() {
                    Ok(id) => services.memory_store.operation(&services.tenant, id).await.map_err(ui)?,
                    Err(_) => None,
                };
                match record {
                    Some(r) => (
                        zorix_memory::text::cut(r.doc["prompt"].as_str().unwrap_or(""), 120),
                        cut(&r.text, 260),
                        Some(r.execution_id.to_string()),
                        Some(r.state.clone()),
                    ),
                    None => continue,
                }
            }
        };
        hits.push(Hit {
            title,
            text,
            keyword_rank: hit.keyword_rank,
            vector_rank: hit.vector_rank,
            cosine: hit.cosine,
            execution,
            state,
        });
    }
    Ok(Found { hits, degraded: found.degraded })
}

// ---------------------------------------------------------------------------------------------------------------------
// Embeddings

#[derive(Serialize)]
pub struct EmbeddingsState {
    model: Option<String>,
    dimensions: Option<u32>,
    allow_cloud: bool,
    embedded: u64,
    texts: u64,
}

async fn embeddings_state(services: &Services) -> Result<EmbeddingsState, UiError> {
    let settings = services.memory_store.settings(&services.tenant).await.map_err(ui)?;
    let (embedded, texts) = match &settings.embedding_model {
        Some(model) => services.memory_store.vector_coverage(&services.tenant, model).await.map_err(ui)?,
        None => (0, 0),
    };
    Ok(EmbeddingsState {
        model: settings.embedding_model,
        dimensions: settings.embedding_dimensions,
        allow_cloud: settings.allow_cloud_embeddings,
        embedded,
        texts,
    })
}

#[tauri::command]
pub async fn embeddings_info(state: State<'_, AppState>) -> Result<EmbeddingsState, UiError> {
    embeddings_state(state.services()?).await
}

#[derive(Serialize)]
pub struct EmbeddingModel {
    reference: String,
    dimensions: Option<u32>,
    local: bool,
    installed: bool,
}

/// The embedding models on offer. A provider that is down is skipped (and named in `problems`).
#[tauri::command]
pub async fn embedding_models(state: State<'_, AppState>) -> Result<(Vec<EmbeddingModel>, Vec<String>), UiError> {
    let services = state.services()?;
    let (choices, problems) = embedding_choices(&services.router, &CancellationToken::new()).await;
    Ok((
        choices
            .iter()
            .map(|c| EmbeddingModel {
                reference: c.reference(),
                dimensions: c.dimensions,
                local: c.local,
                installed: c.installed,
            })
            .collect(),
        problems.into_iter().map(|(p, why)| if why.starts_with(&p) { why } else { format!("{p}: {why}") }).collect(),
    ))
}

async fn try_embed(services: &Services, model: &str, dimensions: Option<u32>) -> Result<usize, UiError> {
    let request = EmbeddingRequest {
        model: model.into(),
        inputs: vec!["Deploys reach staging first.".into()],
        purpose: EmbedPurpose::Query,
        dimensions,
    };
    let gateway: &dyn ModelGateway = &*services.router;
    let response = gateway.embed(request, CancellationToken::new()).await.map_err(ui)?;
    response.vectors.first().map(Vec::len).ok_or_else(|| UiError::message("The model returned no vector."))
}

/// Choose the model. It is tried first, so a wrong name or a missing key is reported now. A cloud provider needs `cloud`.
#[tauri::command]
pub async fn set_embeddings(
    state: State<'_, AppState>,
    model: String,
    dimensions: Option<u32>,
    cloud: bool,
) -> Result<usize, UiError> {
    let services = state.services()?;
    let (provider, _) = model.split_once('/').ok_or("Name the model as provider/model.")?;
    let entry = services
        .router
        .provider(provider)
        .ok_or_else(|| UiError::message(format!("There is no provider called “{provider}”.")))?;
    if !entry.local && !cloud {
        return Err(UiError::message(format!(
            "{provider} is a cloud provider: the text of your documents and your prompts would be sent to it to be embedded. Tick the box to allow that."
        )));
    }
    let size = try_embed(services, &model, dimensions).await?;
    services
        .memory_store
        .set_settings(
            &services.tenant,
            MemorySettings {
                embedding_model: Some(model),
                embedding_dimensions: dimensions,
                allow_cloud_embeddings: cloud,
            },
        )
        .await
        .map_err(ui)?;
    Ok(size)
}

#[tauri::command]
pub async fn embeddings_off(state: State<'_, AppState>) -> Result<(), UiError> {
    let services = state.services()?;
    services.memory_store.set_settings(&services.tenant, MemorySettings::default()).await.map_err(ui)
}

/// Try the chosen model on a short text. Returns the size of the vector and how long it took, in milliseconds.
#[tauri::command]
pub async fn test_embeddings(state: State<'_, AppState>) -> Result<(usize, u64), UiError> {
    let services = state.services()?;
    let settings = services.memory_store.settings(&services.tenant).await.map_err(ui)?;
    let model = settings.embedding_model.ok_or("No embeddings model is set.")?;
    let started = std::time::Instant::now();
    let size = try_embed(services, &model, settings.embedding_dimensions).await?;
    Ok((size, started.elapsed().as_millis() as u64))
}

/// Embed everything again with the chosen model, in the background. Progress arrives as `knowledge://progress`; the end as
/// `knowledge://embedded`.
#[tauri::command]
pub async fn reembed(app: AppHandle, state: State<'_, AppState>) -> Result<bool, UiError> {
    let services = state.services_arc()?;
    let settings = services.memory_store.settings(&services.tenant).await.map_err(ui)?;
    if settings.embedding_model.is_none() {
        return Err("No embeddings model is set.".into());
    }
    let token = CancellationToken::new();
    {
        let mut job = services.reembed_job.lock();
        if job.is_some() {
            return Ok(false);
        }
        *job = Some(token.clone());
    }
    tokio::spawn(async move {
        let sink = app.clone();
        let progress = move |root: &KnowledgeRoot, p: zorix_memory::index::Progress| {
            let _ = sink.emit("knowledge://progress", json!({ "root": root.id.to_string(), "progress": p }));
        };
        let outcome = services.memory.reembed(&services.tenant, &progress, &token).await;
        services.reembed_job.lock().take();
        let payload = match outcome {
            Ok((_, facts)) => json!({ "facts": facts, "error": null }),
            Err(error) => json!({ "facts": 0, "error": error.to_string() }),
        };
        let _ = app.emit("knowledge://embedded", payload);
    });
    Ok(true)
}

/// Is a re-embed running?
#[tauri::command]
pub async fn reembedding(state: State<'_, AppState>) -> Result<bool, UiError> {
    Ok(state.services()?.reembed_job.lock().is_some())
}
