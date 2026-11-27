//! The settings of the program that are kept in the database: long conversations, how long runs are kept, telemetry and scanned pages.
//! A change is saved and takes effect at once, because the parts that act on it read the same cell.

use tauri::State;
use zorix_core::{AppSettings, Clock, RetentionMode, RetentionReport, RetentionSettings, SettingsStore, SystemClock};

use crate::state::AppState;
use crate::ui_error::{UiError, ui};

/// The settings now.
#[tauri::command]
pub async fn app_settings_get(state: State<'_, AppState>) -> Result<AppSettings, UiError> {
    Ok(state.services()?.app_settings.get())
}

/// Save new settings. Refused, with a sentence that says what to change, when a value makes no sense; nothing is saved then.
#[tauri::command]
pub async fn app_settings_set(state: State<'_, AppState>, settings: AppSettings) -> Result<AppSettings, UiError> {
    let services = state.services()?;
    services.settings.put_settings(&services.tenant, &settings).await.map_err(ui)?;
    services.app_settings.set(settings.clone());
    Ok(settings)
}

/// What a pass of retention would do with these values, without changing anything. The form asks with what it shows, before it is saved.
#[tauri::command]
pub async fn retention_preview(
    state: State<'_, AppState>,
    days: u32,
    mode: RetentionMode,
) -> Result<RetentionReport, UiError> {
    let services = state.services()?;
    let settings = AppSettings { retention: RetentionSettings { days: Some(days), mode }, ..AppSettings::default() };
    settings.validate().map_err(ui)?;
    let report = services.retention.pass(&services.tenant, &settings, SystemClock.now(), true).await.map_err(ui)?;
    Ok(report.unwrap_or_default())
}

/// Do a pass now, with the saved settings. Nothing happens when retention is off.
#[tauri::command]
pub async fn retention_run(state: State<'_, AppState>) -> Result<RetentionReport, UiError> {
    let services = state.services()?;
    let settings = services.app_settings.get();
    let report = services.retention.pass(&services.tenant, &settings, SystemClock.now(), false).await.map_err(ui)?;
    Ok(report.unwrap_or_default())
}

/// What a run would probably cost and how long it would take, from what similar runs took. Starts nothing.
#[tauri::command]
pub async fn estimate_run(
    state: State<'_, AppState>,
    prompt_chars: usize,
    attached_chars: usize,
    model: Option<String>,
    agent: Option<String>,
) -> Result<zorix_core::RunEstimate, UiError> {
    use zorix_core::{AgentSlug, AgentStore, StepPhase};
    let services = state.services()?;
    let agent_id = match agent.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
        Some(slug) => {
            let slug = AgentSlug::new(slug).map_err(ui)?;
            let found = services.hub.agents.agent_by_slug(&services.tenant, &slug).await.map_err(ui)?;
            found.map(|agent| agent.id)
        }
        None => None,
    };
    let model = match (model.as_deref().map(str::trim).filter(|m| !m.is_empty()), agent_id) {
        (Some(chosen), _) => Some(chosen.to_string()),
        (None, Some(_)) => None,
        (None, None) if services.router.resolve("default", StepPhase::Start).is_ok() => Some("default".to_string()),
        (None, None) => Some("demo".to_string()),
    };
    let planned =
        zorix_hub::estimate::Planned { agent: agent_id, model: model.as_deref(), prompt_chars, attached_chars };
    zorix_hub::estimate::estimate(&services.runs, &services.router, &services.tenant, &planned)
        .await
        .map_err(|e| UiError::message(e.to_string()))
}
