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
            model: "demo".into(),
            ..TaskRequest::default()
        },
        limits,
        grants,
        workspace,
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
