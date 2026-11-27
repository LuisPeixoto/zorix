//! The settings of the program that are kept in the database: long conversations, how long runs are kept, telemetry and scanned pages.
//! A change is saved and takes effect at once, because the parts that act on it read the same cell.

use tauri::State;
use zorix_core::{AppSettings, SettingsStore};

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
