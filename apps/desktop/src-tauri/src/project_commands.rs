//! Projects: the screens' commands. They hold no rules; the rules are in `zorix-projects` (ADR-0036, ADR-0037).

use serde::Serialize;
use tauri::State;
use zorix_core::{DeliverableKind, ExecutionId, Project, ProjectEvent, ProjectKey, Task, TaskKey};
use zorix_projects::{
    Board, DeliverableRow, LinkState, Move, NewProject, NewTask, ProjectCard, ProjectFields, TaskDetail, TaskFields,
};

use crate::state::AppState;

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn project_key(value: &str) -> Result<ProjectKey, String> {
    ProjectKey::new(value.trim().to_ascii_uppercase())
}

fn task_key(value: &str) -> Result<TaskKey, String> {
    value.trim().to_ascii_uppercase().parse()
}

/// The projects with their figures.
#[tauri::command]
pub async fn projects_overview(state: State<'_, AppState>, archived: bool) -> Result<Vec<ProjectCard>, String> {
    state.services()?.projects.projects(archived).await.map_err(text)
}

#[tauri::command]
pub async fn project_create(state: State<'_, AppState>, new: NewProject) -> Result<Project, String> {
    state.services()?.projects.create_project(new, None).await.map_err(text)
}

#[tauri::command]
pub async fn project_update(state: State<'_, AppState>, key: String, fields: ProjectFields) -> Result<Project, String> {
    state.services()?.projects.update_project(&project_key(&key)?, fields, None).await.map_err(text)
}

#[tauri::command]
pub async fn project_remove(state: State<'_, AppState>, key: String) -> Result<(), String> {
    state.services()?.projects.remove_project(&project_key(&key)?).await.map_err(text)
}

#[tauri::command]
pub async fn project_board(state: State<'_, AppState>, key: String) -> Result<Board, String> {
    state.services()?.projects.board(&project_key(&key)?).await.map_err(text)
}

/// Ask the project's manager to plan it. Returns the run, which the Runs screen shows.
#[tauri::command]
pub async fn project_ask(
    state: State<'_, AppState>,
    key: String,
    prompt: Option<String>,
) -> Result<ExecutionId, String> {
    state.services()?.projects.ask_manager(&project_key(&key)?, prompt.as_deref()).await.map_err(text)
}

#[tauri::command]
pub async fn project_deliverables(state: State<'_, AppState>, key: String) -> Result<Vec<DeliverableRow>, String> {
    state.services()?.projects.deliverables(&project_key(&key)?).await.map_err(text)
}

#[tauri::command]
pub async fn project_history(
    state: State<'_, AppState>,
    key: String,
    limit: Option<usize>,
) -> Result<Vec<ProjectEvent>, String> {
    state.services()?.projects.history(&project_key(&key)?, None, limit.unwrap_or(100)).await.map_err(text)
}

#[tauri::command]
pub async fn task_add(state: State<'_, AppState>, project: String, new: NewTask) -> Result<Task, String> {
    state.services()?.projects.add_task(&project_key(&project)?, new, None).await.map_err(text)
}

#[tauri::command]
pub async fn task_update(state: State<'_, AppState>, key: String, fields: TaskFields) -> Result<Task, String> {
    state.services()?.projects.update_task(&task_key(&key)?, fields, None).await.map_err(text)
}

#[tauri::command]
pub async fn task_move(state: State<'_, AppState>, key: String, mv: Move) -> Result<Task, String> {
    state.services()?.projects.move_task(&task_key(&key)?, mv, None).await.map_err(text)
}

#[tauri::command]
pub async fn task_restart(state: State<'_, AppState>, key: String, note: Option<String>) -> Result<Task, String> {
    state.services()?.projects.restart_task(&task_key(&key)?, note, None).await.map_err(text)
}

#[tauri::command]
pub async fn task_remove(state: State<'_, AppState>, key: String) -> Result<(), String> {
    state.services()?.projects.remove_task(&task_key(&key)?).await.map_err(text)
}

/// One task, with its history and what could be read about its links.
#[derive(Serialize)]
pub struct TaskPage {
    /// The task and its history.
    #[serde(flatten)]
    pub detail: TaskDetail,
    /// The links, with the state of pull requests that a connection could read.
    pub links: Vec<LinkState>,
}

#[tauri::command]
pub async fn task_detail(state: State<'_, AppState>, key: String) -> Result<TaskPage, String> {
    let services = state.services()?;
    let key = task_key(&key)?;
    let detail = services.projects.task(&key).await.map_err(text)?;
    let links = services.projects.link_states(&key).await.map_err(text)?;
    Ok(TaskPage { detail, links })
}

#[tauri::command]
pub async fn task_comment(state: State<'_, AppState>, key: String, body: String) -> Result<(), String> {
    state.services()?.projects.comment(&task_key(&key)?, &body, None).await.map_err(text)
}

#[tauri::command]
pub async fn task_link(
    state: State<'_, AppState>,
    key: String,
    url: String,
    title: Option<String>,
) -> Result<Task, String> {
    state.services()?.projects.add_link(&task_key(&key)?, &url, title, None).await.map_err(text)
}

#[tauri::command]
pub async fn task_deliver(
    state: State<'_, AppState>,
    key: String,
    kind: DeliverableKind,
    title: String,
    body: String,
) -> Result<Task, String> {
    state
        .services()?
        .projects
        .add_deliverable(&task_key(&key)?, kind, &title, &body, Some(zorix_core::Who::You), None)
        .await
        .map_err(text)
}
