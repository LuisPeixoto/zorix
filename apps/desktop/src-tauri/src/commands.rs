//! The commands the interface can call. They return `Result<T, String>`: the text is shown to the person as it is.
//! The interface decides nothing about permissions or state; it asks, and the engine answers.

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::State;
use tauri::ipc::Channel;
use zorix_core::{
    Action, Approval, ApprovalDecision, ApprovalId, Execution, ExecutionDetail, ExecutionEngine, ExecutionEvent,
    ExecutionId, Grants, Limits, ListFilter, NewExecution, TaskRequest, ToolCallRecord, ToolSpec, tool_grant,
    workspace_grant,
};
use zorix_runtime::{ExecutionSummary, summarize};

use crate::state::{AppState, Services};

fn id_of(text: &str) -> Result<ExecutionId, String> {
    text.parse::<ExecutionId>().map_err(|e| e.to_string())
}

fn services(state: &AppState) -> Result<&Services, String> {
    state.services()
}

/// What the interface shows in its footer and when something is wrong.
#[derive(Serialize)]
pub struct AppInfo {
    version: String,
    data_dir: String,
    /// Set when the engine could not start.
    error: Option<String>,
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        data_dir: state.home.display().to_string(),
        error: state.services.as_ref().err().cloned(),
    }
}

#[tauri::command]
pub fn list_tools(state: State<'_, AppState>) -> Result<Vec<ToolSpec>, String> {
    Ok(services(&state)?.engine.tools().list())
}

#[tauri::command]
pub async fn list_runs(state: State<'_, AppState>, limit: Option<u32>) -> Result<Vec<Execution>, String> {
    let services = services(&state)?;
    let filter = ListFilter { limit: limit.unwrap_or(50).clamp(1, 500) as usize, ..ListFilter::default() };
    services.engine.list(&services.tenant, &filter).await.map_err(|e| e.to_string())
}

/// One execution with everything that belongs to it.
#[derive(Serialize)]
pub struct RunDetail {
    execution: Execution,
    tool_calls: Vec<ToolCallRecord>,
    approvals: Vec<Approval>,
    summary: ExecutionSummary,
}

#[tauri::command]
pub async fn run_detail(state: State<'_, AppState>, id: String) -> Result<RunDetail, String> {
    let services = services(&state)?;
    let id = id_of(&id)?;
    let detail: ExecutionDetail = services.engine.detail(&services.tenant, id).await.map_err(|e| e.to_string())?;
    let events = services.engine.events(&services.tenant, id, 0).await.map_err(|e| e.to_string())?;
    let summary = summarize(&detail, &events);
    Ok(RunDetail { execution: detail.execution, tool_calls: detail.tool_calls, approvals: detail.approvals, summary })
}

/// What the "new run" form sends.
#[derive(Deserialize)]
pub struct StartRun {
    prompt: String,
    #[serde(default)]
    instructions: Option<String>,
    /// Tool ids (or patterns) the run may use.
    #[serde(default)]
    allow_tools: Vec<String>,
    #[serde(default)]
    allow_read: Vec<String>,
    #[serde(default)]
    allow_write: Vec<String>,
    #[serde(default)]
    workspace: Option<String>,
    #[serde(default)]
    max_iterations: Option<u32>,
    /// `provider/model`, `route:name`, an alias or `demo`. Empty means the default.
    #[serde(default)]
    model: Option<String>,
    /// The conversation this run continues. Left out: a new conversation is started.
    #[serde(default)]
    conversation: Option<String>,
    /// What the run may use from memory. Left out: everything.
    #[serde(default)]
    memory: Option<MemoryChoice>,
}

/// The switches of the New run form.
#[derive(Deserialize)]
pub struct MemoryChoice {
    conversation: bool,
    long_term: bool,
    knowledge: bool,
    operations: bool,
}

/// The model reference for a new run: what was chosen, else the default when there is one, else the simulated model.
fn model_for(services: &Services, chosen: Option<&str>) -> Result<String, String> {
    use zorix_core::StepPhase;
    let chosen = chosen.map(str::trim).filter(|m| !m.is_empty());
    let model = match chosen {
        Some(model) => model.to_string(),
        None if services.router.resolve("default", StepPhase::Start).is_ok() => "default".to_string(),
        None => "demo".to_string(),
    };
    // Say now, in words, what a typo or an unknown provider means, instead of failing inside the run.
    services.router.resolve(&model, StepPhase::Start)?;
    Ok(model)
}

#[tauri::command]
pub async fn start_run(state: State<'_, AppState>, request: StartRun) -> Result<Execution, String> {
    let services = services(&state)?;
    let mut grants = Grants::none();
    for tool in &request.allow_tools {
        grants = grants.with(tool_grant(tool)?);
    }
    for pattern in &request.allow_read {
        grants = grants.with(workspace_grant(Action::Read, pattern)?);
    }
    for pattern in &request.allow_write {
        grants = grants.with(workspace_grant(Action::Write, pattern)?);
    }
    let workspace = match request.workspace.as_deref().map(str::trim).filter(|w| !w.is_empty()) {
        Some(path) => {
            Some(std::fs::canonicalize(path).map_err(|e| format!("The workspace {path} cannot be used: {e}"))?)
        }
        None => None,
    };
    let mut limits = Limits::default();
    if let Some(n) = request.max_iterations {
        limits.max_iterations = n;
    }
    let new = NewExecution {
        tenant: Some(services.tenant.clone()),
        request: TaskRequest {
            instructions: request.instructions.filter(|text| !text.trim().is_empty()),
            prompt: request.prompt,
            model: model_for(services, request.model.as_deref())?,
            ..TaskRequest::default()
        },
        limits,
        grants,
        workspace,
        // The desktop always has a conversation, so "Continue" can pick it up.
        conversation: Some(match request.conversation.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
            Some(id) => zorix_core::ConversationId::new(id).map_err(|e| e.to_string())?,
            None => zorix_core::ConversationId::new(format!("chat-{}", &ExecutionId::new().to_string()[24..]))
                .map_err(|e| e.to_string())?,
        }),
        memory: match request.memory {
            Some(m) => zorix_core::MemoryOptions {
                conversation: m.conversation,
                long_term: m.long_term,
                knowledge: if m.knowledge { zorix_core::RecallMode::Auto } else { zorix_core::RecallMode::Off },
                operations: if m.operations { zorix_core::RecallMode::Auto } else { zorix_core::RecallMode::Off },
            },
            None => zorix_core::MemoryOptions::default(),
        },
        ..NewExecution::default()
    };
    let created = services.engine.create(new).await.map_err(|e| e.to_string())?;
    services.engine.start(&services.tenant, created.id).await.map_err(|e| e.to_string())
}

/// Send the events of an execution to the interface, first the ones already stored (after `after_seq`), then live ones,
/// until the execution finishes or `stop_watch` is called.
#[tauri::command]
pub async fn watch_run(
    state: State<'_, AppState>,
    id: String,
    after_seq: u64,
    on_event: Channel<ExecutionEvent>,
) -> Result<(), String> {
    let services = services(&state)?;
    let id = id_of(&id)?;
    let mut stream = services.engine.subscribe(&services.tenant, id, after_seq).await.map_err(|e| e.to_string())?;
    let stop = state.watch(id);
    loop {
        tokio::select! {
            _ = stop.cancelled() => break,
            next = stream.next() => match next {
                Some(event) => {
                    if on_event.send(event).is_err() {
                        break;
                    }
                }
                None => break,
            },
        }
    }
    Ok(())
}

#[tauri::command]
pub fn stop_watch(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.unwatch(id_of(&id)?);
    Ok(())
}

#[tauri::command]
pub async fn decide_run(
    state: State<'_, AppState>,
    id: String,
    approval_id: String,
    approve: bool,
    note: Option<String>,
    call_hash: Option<String>,
) -> Result<Execution, String> {
    let services = services(&state)?;
    let approval = approval_id.parse::<ApprovalId>().map_err(|e| e.to_string())?;
    let decision = ApprovalDecision {
        approve,
        decided_by: "desktop".into(),
        note: note.filter(|n| !n.trim().is_empty()),
        call_hash,
        automatic: false,
    };
    services.engine.decide(&services.tenant, id_of(&id)?, approval, decision).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cancel_run(state: State<'_, AppState>, id: String, reason: Option<String>) -> Result<Execution, String> {
    let services = services(&state)?;
    services.engine.cancel(&services.tenant, id_of(&id)?, reason).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn pause_run(state: State<'_, AppState>, id: String) -> Result<Execution, String> {
    let services = services(&state)?;
    services.engine.pause(&services.tenant, id_of(&id)?).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn resume_run(state: State<'_, AppState>, id: String) -> Result<Execution, String> {
    let services = services(&state)?;
    services.engine.resume(&services.tenant, id_of(&id)?).await.map_err(|e| e.to_string())
}

/// One provider as the settings screen shows it. Never carries a key.
#[derive(Serialize)]
pub struct ProviderRow {
    name: String,
    kind: String,
    url: String,
    local: bool,
    enabled: bool,
    /// `not_needed`, `present`, `missing` or `unavailable`.
    key: String,
    key_note: Option<String>,
}

#[tauri::command]
pub async fn list_providers(state: State<'_, AppState>) -> Result<Vec<ProviderRow>, String> {
    let services = services(&state)?;
    let secrets = services.secrets.clone();
    let config = services.config.clone();
    // The keychain can be slow; keep it off the async threads.
    tokio::task::spawn_blocking(move || {
        zorix_providers::provider_statuses(&config, &secrets)
            .into_iter()
            .map(|s| {
                use zorix_providers::KeyStatus::*;
                let (key, key_note) = match s.key {
                    NotNeeded => ("not_needed", None),
                    Present => ("present", None),
                    Missing => ("missing", None),
                    Unavailable(why) => ("unavailable", Some(why)),
                };
                ProviderRow {
                    name: s.name,
                    kind: s.kind.as_str().into(),
                    url: s.url,
                    local: s.local,
                    enabled: s.enabled,
                    key: key.into(),
                    key_note,
                }
            })
            .collect()
    })
    .await
    .map_err(|e| e.to_string())
}

fn known_provider(services: &Services, name: &str) -> Result<(), String> {
    zorix_providers::secrets::validate_name(name).map_err(|e| e.to_string())?;
    if services.config.instances().iter().any(|p| p.name == name) {
        Ok(())
    } else {
        Err(format!("There is no provider called “{name}”."))
    }
}

/// The key goes from the field straight to the keychain. It is not stored in the interface, logged or returned.
#[tauri::command]
pub async fn set_provider_key(state: State<'_, AppState>, name: String, key: String) -> Result<(), String> {
    let services = services(&state)?;
    known_provider(services, &name)?;
    let key = key.trim().to_string();
    zorix_providers::secrets::validate_value(&key).map_err(|e| e.to_string())?;
    let secrets = services.secrets.clone();
    tokio::task::spawn_blocking(move || secrets.set(&name, &zorix_core::Secret::new(key)).map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn remove_provider_key(state: State<'_, AppState>, name: String) -> Result<bool, String> {
    let services = services(&state)?;
    known_provider(services, &name)?;
    let secrets = services.secrets.clone();
    tokio::task::spawn_blocking(move || secrets.remove(&name).map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

/// What a provider offers. Asks the provider (no conversation is sent) and teaches the router what it finds.
#[tauri::command]
pub async fn test_provider(state: State<'_, AppState>, name: String) -> Result<usize, String> {
    let services = services(&state)?;
    known_provider(services, &name)?;
    let found = zorix_providers::discover_all(&services.router, &tokio_util::sync::CancellationToken::new()).await;
    match found.into_iter().find(|d| d.provider == name) {
        Some(d) => d.models.map(|m| m.len()),
        None => Err(format!("{name} is turned off or cannot list its models.")),
    }
}

/// A model for the picker.
#[derive(Serialize)]
pub struct ModelRow {
    reference: String,
    name: String,
    context_window: u64,
    tools: bool,
    vision: bool,
    local: bool,
    price_known: bool,
}

/// The picker's choices: the aliases and routes from the configuration, then the known models.
#[derive(Serialize)]
pub struct ModelChoices {
    aliases: Vec<String>,
    routes: Vec<String>,
    models: Vec<ModelRow>,
    /// What “default” resolves to, or why it does not.
    default: Result<String, String>,
}

#[tauri::command]
pub fn list_models(state: State<'_, AppState>) -> Result<ModelChoices, String> {
    use zorix_core::StepPhase;
    let services = services(&state)?;
    let router = &services.router;
    let mut models: Vec<ModelRow> = router
        .known_models()
        .into_iter()
        .filter(|m| m.tools)
        .map(|m| ModelRow {
            reference: format!("{}/{}", m.provider, m.id),
            name: if m.display_name.is_empty() { m.id.clone() } else { m.display_name.clone() },
            context_window: m.context_window,
            tools: m.tools,
            vision: m.vision,
            local: router.provider(&m.provider).is_some_and(|p| p.local),
            price_known: m.price.is_some(),
        })
        .collect();
    models.sort_by(|a, b| a.reference.cmp(&b.reference));
    let default = router
        .resolve("default", StepPhase::Start)
        .map(|r| r.candidates.first().map(|c| c.label()).unwrap_or_default());
    Ok(ModelChoices {
        aliases: services.config.alias.keys().cloned().collect(),
        routes: services.config.routes.iter().map(|r| r.name.clone()).collect(),
        models,
        default,
    })
}
