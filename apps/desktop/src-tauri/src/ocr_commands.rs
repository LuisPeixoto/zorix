//! Reading scanned PDFs: the switch on the Documents screen. Turning it on downloads two small models once (with progress), checks them and loads
//! the engine; only then is the setting saved, so it is never on without models. Turning it off unloads the engine and keeps the files.

use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, State};
use zorix_core::{MemoryStore, SettingsStore};
use zorix_hub::ocr;

use crate::knowledge_commands::spawn_sync;
use crate::state::{AppState, Services};
use crate::ui_error::{UiError, ui};

/// Where reading scans stands.
#[derive(Serialize)]
pub struct OcrStatus {
    /// The setting.
    enabled: bool,
    /// `off`, `ready`, `missing` (on, but the models are not there) or `installing`.
    state: &'static str,
    /// Bytes received of the download, while there is one.
    done: u64,
    /// Bytes the download will have.
    total: u64,
}

fn status(services: &Services) -> OcrStatus {
    let enabled = services.app_settings.get().knowledge.ocr;
    let total: u64 = ocr::MODELS.iter().map(|m| m.size).sum();
    if let Some(progress) = *services.ocr_installing.lock() {
        return OcrStatus { enabled, state: "installing", done: progress.done, total: progress.total };
    }
    let state = match (enabled, services.memory.has_ocr()) {
        (false, _) => "off",
        (true, true) => "ready",
        (true, false) => "missing",
    };
    OcrStatus { enabled, state, done: 0, total }
}

/// Where reading scans stands now.
#[tauri::command]
pub async fn ocr_status(state: State<'_, AppState>) -> Result<OcrStatus, UiError> {
    Ok(status(state.services()?))
}

/// Turn reading scans on or off. Turning it on can take a while the first time (the models are downloaded) and reports with `ocr://progress`;
/// when it is done, the folders that had scans left out are read again.
#[tauri::command]
pub async fn ocr_set(app: AppHandle, state: State<'_, AppState>, enabled: bool) -> Result<OcrStatus, UiError> {
    let services = state.services_arc()?;
    let mut settings = services.app_settings.get();
    if enabled {
        {
            let mut installing = services.ocr_installing.lock();
            if installing.is_some() {
                return Err(UiError::message("The models are already being downloaded."));
            }
            *installing = Some(ocr::Progress { done: 0, total: ocr::MODELS.iter().map(|m| m.size).sum() });
        }
        let (sink, shared) = (app.clone(), services.clone());
        let outcome = ocr::enable(&services.memory, &state.home, &move |p| {
            *shared.ocr_installing.lock() = Some(p);
            let _ = sink.emit("ocr://progress", json!({ "done": p.done, "total": p.total }));
        })
        .await;
        *services.ocr_installing.lock() = None;
        outcome.map_err(|e| UiError::message(e.to_string()))?;
    } else {
        ocr::attach(&services.memory, &state.home, false).await.map_err(|e| UiError::message(e.to_string()))?;
    }
    settings.knowledge.ocr = enabled;
    services.settings.put_settings(&services.tenant, &settings).await.map_err(ui)?;
    services.app_settings.set(settings);
    if enabled {
        // Scans that were left out for lack of a reader are read now, one folder after the other.
        for root in services.memory_store.all_roots(&services.tenant).await.map_err(ui)? {
            if root.enabled && services.memory_store.scan_counts(root.id).await.map_err(ui)?.unread > 0 {
                spawn_sync(app.clone(), &services, root);
            }
        }
    }
    Ok(status(&services))
}
