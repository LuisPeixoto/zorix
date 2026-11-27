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
