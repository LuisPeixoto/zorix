//! The workforce: the screens' commands. They hold no rules; the rules are in `zorix-workforce` (ADR-0038, ADR-0039).

use tauri::State;
use zorix_core::{AgentSlug, OrgReport, OrgSettings, Worker};
use zorix_workforce::{ActivityLine, NewWorker, Notice, Overview, SettingsFields, WorkerDetail, WorkerFields};

use crate::state::AppState;

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn slug(value: &str) -> Result<AgentSlug, String> {
    AgentSlug::new(value.trim()).map_err(text)
}

/// The organisation: workers with their figures, and the numbers above the chart.
#[tauri::command]
pub async fn workforce_overview(state: State<'_, AppState>) -> Result<Overview, String> {
    state.services()?.workforce.overview().await.map_err(text)
}

/// One worker's page.
#[tauri::command]
pub async fn worker_detail(state: State<'_, AppState>, agent: String) -> Result<WorkerDetail, String> {
    state.services()?.workforce.detail(&slug(&agent)?).await.map_err(text)
}

#[tauri::command]
pub async fn worker_hire(state: State<'_, AppState>, new: NewWorker) -> Result<Worker, String> {
    state.services()?.workforce.hire(new, "you").await.map_err(text)
}

#[tauri::command]
pub async fn worker_edit(state: State<'_, AppState>, agent: String, fields: WorkerFields) -> Result<Worker, String> {
    state.services()?.workforce.edit(&slug(&agent)?, fields, "you").await.map_err(text)
}

#[tauri::command]
pub async fn worker_fire(state: State<'_, AppState>, agent: String) -> Result<(), String> {
    state.services()?.workforce.fire(&slug(&agent)?, "you").await.map_err(text)
}

#[tauri::command]
pub async fn workforce_settings_save(
    state: State<'_, AppState>,
    fields: SettingsFields,
) -> Result<OrgSettings, String> {
    state.services()?.workforce.save_settings(fields, "you").await.map_err(text)
}

/// What the workers did, newest first.
#[tauri::command]
pub async fn workforce_activity(state: State<'_, AppState>, limit: Option<usize>) -> Result<Vec<ActivityLine>, String> {
    state.services()?.workforce.activity(limit.unwrap_or(100)).await.map_err(text)
}

/// What is worth a person's eye: plans and reports, proposals, calls that ran without asking and budget warnings.
#[tauri::command]
pub async fn workforce_notices(state: State<'_, AppState>) -> Result<Vec<Notice>, String> {
    state.services()?.workforce.notices().await.map_err(text)
}

#[tauri::command]
pub async fn workforce_mark_seen(state: State<'_, AppState>) -> Result<(), String> {
    state.services()?.workforce.mark_seen().await.map_err(text)
}

/// Ask a manager to plan the week. The run follows its level and budget.
#[tauri::command]
pub async fn worker_plan(
    state: State<'_, AppState>,
    agent: String,
    prompt: Option<String>,
) -> Result<OrgReport, String> {
    state.services()?.workforce.plan_week(&slug(&agent)?, prompt.as_deref()).await.map_err(text)
}

/// Ask a manager for the weekly report.
#[tauri::command]
pub async fn worker_report(
    state: State<'_, AppState>,
    agent: String,
    prompt: Option<String>,
) -> Result<OrgReport, String> {
    state.services()?.workforce.write_report(&slug(&agent)?, prompt.as_deref()).await.map_err(text)
}
