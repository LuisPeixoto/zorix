//! The commands behind the Agents screens: agents and their versions, skills, evaluations, conversations and live text.
//! Like every command they return plain text errors and decide nothing the hub, the store or the engine does not decide.

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::State;
use tauri::ipc::Channel;
use tokio_util::sync::CancellationToken;
use zorix_agents::diff::Change as Line;
use zorix_agents::evals::{EvalOptions, Evaluator, compare};
use zorix_agents::{AgentAsk, AgentCard};
use zorix_core::{
    Agent, AgentConversation, AgentSpec, AgentStore, AgentVersion, Approval, EvalCase, EvalRun, EvalRunId, Execution,
    ExecutionEngine, ExecutionId, ListFilter, LiveEvent, SkillRecord, StoredCase, ToolCallRecord,
};

use crate::state::{AppState, Services};

fn services(state: &AppState) -> Result<&Services, String> {
    state.services()
}

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

// ---------------------------------------------------------------------------------------------------------------------
// Agents

#[tauri::command]
pub async fn agents_list(state: State<'_, AppState>, include_archived: Option<bool>) -> Result<Vec<AgentCard>, String> {
    services(&state)?.hub.agent_cards(include_archived.unwrap_or(false)).await.map_err(text)
}

/// An agent with one of its versions and the skills that version pins.
#[derive(Serialize)]
pub struct AgentDetail {
    agent: Agent,
    version: AgentVersion,
    skills: Vec<SkillRecord>,
    cases: usize,
}

#[tauri::command]
pub async fn agent_get(state: State<'_, AppState>, slug: String, version: Option<u32>) -> Result<AgentDetail, String> {
    let services = services(&state)?;
    let (agent, found) = services.hub.agent_version(&slug, version).await.map_err(text)?;
    let mut skills = Vec::new();
    for pin in &found.spec.skills {
        if let Some(record) =
            services.hub.agents.skill_version(&services.tenant, &pin.name, &pin.hash).await.map_err(text)?
        {
            skills.push(record);
        }
    }
    let cases = services.hub.agents.eval_cases(&services.tenant, agent.id).await.map_err(text)?.len();
    Ok(AgentDetail { agent, version: found, skills, cases })
}

/// What saving or importing did.
#[derive(Serialize)]
pub struct Saved {
    slug: String,
    version: u32,
    created: bool,
    changed: bool,
}

impl From<zorix_hub::agents::Imported> for Saved {
    fn from(i: zorix_hub::agents::Imported) -> Self {
        Saved { slug: i.agent.slug.to_string(), version: i.version.version, created: i.created, changed: i.changed }
    }
}

#[tauri::command]
pub async fn agent_save(state: State<'_, AppState>, spec: AgentSpec, note: Option<String>) -> Result<Saved, String> {
    let services = services(&state)?;
    spec.validate().map_err(text)?;
    services.hub.save_spec(spec, note.filter(|n| !n.trim().is_empty())).await.map(Into::into).map_err(text)
}

#[tauri::command]
pub async fn agent_import(state: State<'_, AppState>, file: String, rename: Option<String>) -> Result<Saved, String> {
    let rename = rename.filter(|r| !r.trim().is_empty());
    services(&state)?
        .hub
        .import_agent(&file, rename.as_deref(), Some("imported".into()))
        .await
        .map(Into::into)
        .map_err(text)
}

#[tauri::command]
pub async fn agent_export(state: State<'_, AppState>, slug: String, version: Option<u32>) -> Result<String, String> {
    services(&state)?.hub.export_agent(&slug, version).await.map_err(text)
}

#[tauri::command]
pub async fn agent_versions(state: State<'_, AppState>, slug: String) -> Result<Vec<AgentVersion>, String> {
    let services = services(&state)?;
    let (agent, _) = services.hub.agent(&slug).await.map_err(text)?;
    // Newest first, as the store keeps them.
    services.hub.agents.versions(&services.tenant, agent.id).await.map_err(text)
}

/// One line of a diff between versions.
#[derive(Serialize)]
pub struct DiffRow {
    change: &'static str,
    text: String,
}

#[tauri::command]
pub async fn agent_diff(state: State<'_, AppState>, slug: String, from: u32, to: u32) -> Result<Vec<DiffRow>, String> {
    let lines = services(&state)?.hub.diff_agent(&slug, from, to).await.map_err(text)?;
    Ok(lines
        .into_iter()
        .map(|l| DiffRow {
            change: match l.kind {
                Line::Same => "same",
                Line::Added => "added",
                Line::Removed => "removed",
            },
            text: l.text,
        })
        .collect())
}

#[tauri::command]
pub async fn agent_rollback(state: State<'_, AppState>, slug: String, to: u32) -> Result<AgentVersion, String> {
    services(&state)?.hub.rollback_agent(&slug, to).await.map_err(text)
}

#[tauri::command]
pub async fn agent_archive(state: State<'_, AppState>, slug: String, archived: bool) -> Result<(), String> {
    services(&state)?.hub.archive_agent(&slug, archived).await.map(|_| ()).map_err(text)
}

#[tauri::command]
pub async fn agent_remove(state: State<'_, AppState>, slug: String) -> Result<(), String> {
    services(&state)?.hub.remove_agent(&slug).await.map_err(text)
}

/// A starter agent, as the gallery offers it.
#[derive(Serialize)]
pub struct Template {
    id: String,
    name: String,
    description: String,
    emoji: String,
    color: String,
    skills: Vec<String>,
    cases: usize,
    installed: bool,
}

#[tauri::command]
pub async fn agent_templates(state: State<'_, AppState>) -> Result<Vec<Template>, String> {
    let all = services(&state)?.hub.templates().await.map_err(text)?;
    Ok(all
        .into_iter()
        .map(|t| Template {
            id: t.id,
            name: t.name,
            description: t.description,
            emoji: t.emoji,
            color: t.color,
            skills: t.skills,
            cases: t.cases,
            installed: t.installed,
        })
        .collect())
}

#[tauri::command]
pub async fn agent_add_template(
    state: State<'_, AppState>,
    id: String,
    rename: Option<String>,
) -> Result<Saved, String> {
    let rename = rename.filter(|r| !r.trim().is_empty());
    services(&state)?.hub.add_template(&id, rename.as_deref()).await.map(|d| d.imported.into()).map_err(text)
}

// ---------------------------------------------------------------------------------------------------------------------
// Skills

#[tauri::command]
pub async fn skills_list(state: State<'_, AppState>) -> Result<Vec<SkillRecord>, String> {
    services(&state)?.hub.skills_list().await.map_err(text)
}

#[tauri::command]
pub async fn skill_import(state: State<'_, AppState>, path: String) -> Result<SkillRecord, String> {
    services(&state)?.hub.import_skill(std::path::Path::new(&path)).await.map_err(text)
}

#[tauri::command]
pub async fn skill_remove(state: State<'_, AppState>, name: String) -> Result<(), String> {
    services(&state)?.hub.remove_skill(&name).await.map_err(text)
}

/// Read one text file of a skill (what the screen shows when a file is opened).
#[tauri::command]
pub async fn skill_file(state: State<'_, AppState>, hash: String, path: String) -> Result<String, String> {
    let library = services(&state)?.hub.skills.clone();
    tokio::task::spawn_blocking(move || {
        let bytes = library.read(&hash, &path, 256 * 1024).map_err(text)?;
        String::from_utf8(bytes).map_err(|_| "That file is not text.".to_string())
    })
    .await
    .map_err(text)?
}

// ---------------------------------------------------------------------------------------------------------------------
// Evaluations

#[tauri::command]
pub async fn eval_cases(state: State<'_, AppState>, slug: String) -> Result<Vec<StoredCase>, String> {
    let services = services(&state)?;
    let (agent, _) = services.hub.agent(&slug).await.map_err(text)?;
    services.hub.agents.eval_cases(&services.tenant, agent.id).await.map_err(text)
}

#[tauri::command]
pub async fn eval_put_case(state: State<'_, AppState>, slug: String, case: EvalCase) -> Result<StoredCase, String> {
    let services = services(&state)?;
    case.validate().map_err(text)?;
    let (agent, _) = services.hub.agent(&slug).await.map_err(text)?;
    let now = zorix_core::Clock::now(&zorix_core::SystemClock);
    services.hub.agents.put_eval_case(&services.tenant, agent.id, case, now).await.map_err(text)
}

#[tauri::command]
pub async fn eval_remove_case(state: State<'_, AppState>, slug: String, name: String) -> Result<(), String> {
    let services = services(&state)?;
    let (agent, _) = services.hub.agent(&slug).await.map_err(text)?;
    if services.hub.agents.remove_eval_case(&services.tenant, agent.id, &name).await.map_err(text)? {
        Ok(())
    } else {
        Err(format!("There is no case called {name}."))
    }
}

#[tauri::command]
pub async fn eval_runs(state: State<'_, AppState>, slug: String) -> Result<Vec<EvalRun>, String> {
    let services = services(&state)?;
    let (agent, _) = services.hub.agent(&slug).await.map_err(text)?;
    services.hub.agents.eval_runs(&services.tenant, agent.id, 30).await.map_err(text)
}

#[tauri::command]
pub fn eval_running(state: State<'_, AppState>, slug: String) -> Result<bool, String> {
    Ok(services(&state)?.evals.lock().contains_key(&slug))
}

/// What the Evals tab asks for.
#[derive(Deserialize)]
pub struct StartEval {
    slug: String,
    #[serde(default)]
    version: Option<u32>,
    /// Models to try, one run each. Empty: the agent's own.
    #[serde(default)]
    models: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    offline: bool,
}

/// Start an evaluation in the background; the screen reads the runs again while it goes.
#[tauri::command]
pub async fn eval_start(state: State<'_, AppState>, request: StartEval) -> Result<(), String> {
    let services = state.services_arc()?;
    let (agent, version) = services.hub.agent_version(&request.slug, request.version).await.map_err(text)?;
    let cases = services.hub.agents.eval_cases(&services.tenant, agent.id).await.map_err(text)?;
    if cases.is_empty() {
        return Err(format!("{} has no evaluation cases yet.", agent.slug));
    }
    let cancel = CancellationToken::new();
    {
        let mut running = services.evals.lock();
        if running.contains_key(&request.slug) {
            return Err(format!("An evaluation of {} is already running.", request.slug));
        }
        running.insert(request.slug.clone(), cancel.clone());
    }
    let judge: Arc<dyn zorix_core::ModelGateway> = services.router.clone();
    let engine: Arc<dyn ExecutionEngine> = Arc::new(services.engine.clone());
    let evaluator =
        Evaluator { engine, store: services.hub.agents.clone(), resolver: services.router.clone(), judge: Some(judge) };
    let models: Vec<Option<String>> =
        if request.models.is_empty() { vec![None] } else { request.models.iter().cloned().map(Some).collect() };
    tokio::spawn(async move {
        for model in models {
            if cancel.is_cancelled() {
                break;
            }
            let options = EvalOptions {
                model,
                offline: request.offline,
                tags: request.tags.clone(),
                concurrency: 2,
                ..EvalOptions::default()
            };
            if let Err(error) = evaluator.run(&agent, &version, cases.clone(), options, cancel.clone(), None).await {
                tracing::warn!(%error, "an evaluation could not run");
                break;
            }
        }
        services.evals.lock().remove(&request.slug);
    });
    Ok(())
}

#[tauri::command]
pub fn eval_cancel(state: State<'_, AppState>, slug: String) -> Result<(), String> {
    if let Some(token) = services(&state)?.evals.lock().get(&slug) {
        token.cancel();
    }
    Ok(())
}

/// The change from one run to another, for the comparison.
#[derive(Serialize)]
pub struct CompareRow {
    case: String,
    change: String,
    cost_delta_micros: i64,
    steps_delta: i64,
    score_delta: Option<i64>,
}

#[tauri::command]
pub async fn eval_compare(
    state: State<'_, AppState>,
    before: String,
    after: String,
) -> Result<Vec<CompareRow>, String> {
    let services = services(&state)?;
    let find = |id: String| {
        let services = &*services;
        async move {
            let id: EvalRunId = id.parse().map_err(|_| format!("“{id}” is not an evaluation run."))?;
            services
                .hub
                .agents
                .eval_run(&services.tenant, id)
                .await
                .map_err(text)?
                .ok_or_else(|| "That evaluation run no longer exists.".to_string())
        }
    };
    let (a, b) = (find(before).await?, find(after).await?);
    Ok(compare(&a, &b)
        .into_iter()
        .map(|r| CompareRow {
            case: r.case,
            change: format!("{:?}", r.change),
            cost_delta_micros: r.cost_delta_micros,
            steps_delta: r.steps_delta,
            score_delta: r.score_delta,
        })
        .collect())
}

// ---------------------------------------------------------------------------------------------------------------------
// Conversations

/// What the chat sends.
#[derive(Deserialize)]
pub struct AskAgent {
    slug: String,
    #[serde(default)]
    version: Option<u32>,
    prompt: String,
    #[serde(default)]
    input: Option<serde_json::Value>,
    #[serde(default)]
    model: Option<String>,
    /// The conversation this continues. Left out: a new one starts.
    #[serde(default)]
    conversation: Option<String>,
    /// Less memory than the agent has. Left out: all of it.
    #[serde(default)]
    memory: Option<AskMemory>,
}

/// The switches of the New run form, when a run is made by an agent.
#[derive(Deserialize)]
pub struct AskMemory {
    conversation: bool,
    long_term: bool,
    knowledge: bool,
    operations: bool,
}

#[tauri::command]
pub async fn agent_ask(state: State<'_, AppState>, request: AskAgent) -> Result<Execution, String> {
    let services = services(&state)?;
    let conversation = match request.conversation.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        Some(id) => zorix_core::ConversationId::new(id).map_err(text)?,
        None => {
            zorix_core::ConversationId::new(format!("chat-{}", &ExecutionId::new().to_string()[24..])).map_err(text)?
        }
    };
    let ask = AgentAsk {
        prompt: request.prompt,
        input: request.input,
        model: request.model.map(|m| m.trim().to_string()).filter(|m| !m.is_empty()),
        conversation: Some(conversation),
        memory: request.memory.map(|m| zorix_core::MemoryOptions {
            conversation: m.conversation,
            long_term: m.long_term,
            knowledge: if m.knowledge { zorix_core::RecallMode::Auto } else { zorix_core::RecallMode::Off },
            operations: if m.operations { zorix_core::RecallMode::Auto } else { zorix_core::RecallMode::Off },
        }),
        ..AgentAsk::default()
    };
    let new =
        services.hub.prepare_agent_run(&request.slug, request.version, ask, &*services.router).await.map_err(text)?;
    let created = services.engine.create(new).await.map_err(text)?;
    services.engine.start(&services.tenant, created.id).await.map_err(text)
}

#[tauri::command]
pub async fn agent_threads(state: State<'_, AppState>, slug: String) -> Result<Vec<AgentConversation>, String> {
    let services = services(&state)?;
    let (agent, _) = services.hub.agent(&slug).await.map_err(text)?;
    services.hub.agents.conversations(&services.tenant, agent.id, 50).await.map_err(text)
}

/// One run of a conversation, with what the chat draws from it.
#[derive(Serialize)]
pub struct ThreadRun {
    execution: Execution,
    tool_calls: Vec<ToolCallRecord>,
    approvals: Vec<Approval>,
}

#[tauri::command]
pub async fn thread_runs(
    state: State<'_, AppState>,
    slug: String,
    conversation: String,
) -> Result<Vec<ThreadRun>, String> {
    let services = services(&state)?;
    let (agent, _) = services.hub.agent(&slug).await.map_err(text)?;
    let filter = ListFilter {
        conversation: Some(zorix_core::ConversationId::new(conversation).map_err(text)?),
        agent: Some(agent.id),
        limit: 100,
        ..ListFilter::default()
    };
    let mut runs = services.engine.list(&services.tenant, &filter).await.map_err(text)?;
    runs.reverse();
    let mut out = Vec::new();
    for run in runs {
        let detail = services.engine.detail(&services.tenant, run.id).await.map_err(text)?;
        out.push(ThreadRun { execution: detail.execution, tool_calls: detail.tool_calls, approvals: detail.approvals });
    }
    Ok(out)
}

/// What the chat gets while an answer is being written: pieces joined for 30 ms, so the screen draws a few times a second
/// instead of once per word.
#[derive(Serialize, Clone)]
pub struct LiveBatch {
    execution: String,
    text: String,
    /// Forget what was drawn before this batch: another model took over.
    reset: bool,
}

#[tauri::command]
pub async fn watch_live(state: State<'_, AppState>, id: String, on_text: Channel<LiveBatch>) -> Result<(), String> {
    let services = services(&state)?;
    let id: ExecutionId = id.parse().map_err(text)?;
    let mut stream = services.engine.live(&services.tenant, id).await.map_err(text)?;
    let stop = state.watch_live(id);
    let mut pending = String::new();
    let mut reset = false;
    let mut tick = tokio::time::interval(Duration::from_millis(30));
    let execution = id.to_string();
    loop {
        tokio::select! {
            _ = stop.cancelled() => break,
            _ = tick.tick() => {
                if reset || !pending.is_empty() {
                    let batch = LiveBatch { execution: execution.clone(), text: std::mem::take(&mut pending), reset };
                    reset = false;
                    if on_text.send(batch).is_err() {
                        break;
                    }
                }
            }
            next = stream.next() => match next {
                Some(LiveEvent::Text { delta, .. }) => pending.push_str(&delta),
                Some(LiveEvent::Reset { .. }) => {
                    pending.clear();
                    reset = true;
                }
                None => break,
            },
        }
    }
    Ok(())
}

#[tauri::command]
pub fn stop_live(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.unwatch_live(id.parse().map_err(text)?);
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------------
// Links

/// Open an address in the person's browser. Only web and mail addresses.
#[tauri::command]
pub fn open_external(url: String) -> Result<(), String> {
    let lower = url.to_ascii_lowercase();
    let ok = (lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:"))
        && !url.chars().any(|c| c.is_control() || c.is_whitespace());
    if !ok {
        return Err("Only web and mail addresses can be opened.".into());
    }
    let mut command = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(&url);
        c
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler").arg(&url);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(&url);
        c
    };
    command.spawn().map(|_| ()).map_err(|e| format!("The browser could not be opened: {e}"))
}
