//! Workflows: the host that starts them by itself, and its settings. The screens' own commands are added with them.

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::settings::Settings;
use crate::state::{AppState, Services};

/// Whether workflows start by themselves, and how webhooks are set.
#[derive(Serialize)]
pub struct HostInfo {
    /// Whether the host is running (schedules and events work while it is).
    pub running: bool,
    /// Whether the person turned webhooks on.
    pub webhooks: bool,
    /// The address the listener is bound to, when it is.
    pub listening: Option<String>,
    /// The address saved in Settings.
    pub address: String,
}

async fn info(services: &Services, home: &std::path::Path) -> HostInfo {
    let settings = Settings::load(home);
    let host = services.host.lock().await;
    HostInfo {
        running: host.is_some(),
        webhooks: settings.webhooks,
        listening: host.as_ref().and_then(|h| h.webhook_addr()).map(|a| a.to_string()),
        address: settings.webhook_address,
    }
}

#[tauri::command]
pub async fn workflow_host(state: State<'_, AppState>) -> Result<HostInfo, String> {
    let services = state.services()?;
    Ok(info(services, &state.home).await)
}

/// Turn webhooks on or off, or move them to another address. The host restarts; nothing it was running is lost, because runs
/// live in the store.
#[tauri::command]
pub async fn workflow_settings_set(
    state: State<'_, AppState>,
    webhooks: bool,
    address: String,
) -> Result<HostInfo, String> {
    let services = state.services_arc()?;
    let wanted = Settings { webhooks, webhook_address: address.trim().to_string() };
    let listen = wanted.listen()?;
    let mut host = services.host.lock().await;
    if let Some(old) = host.take() {
        old.shutdown().await;
    }
    let started = services.workflows.start_host(services.secrets.clone(), listen).await;
    match started {
        Ok(new) => {
            *host = Some(Arc::new(new));
            wanted.save(&state.home)?;
        }
        Err(error) => {
            // The old setting stays: start again with it so schedules keep working.
            let previous = Settings::load(&state.home).listen().ok().flatten();
            if let Ok(again) = services.workflows.start_host(services.secrets.clone(), previous).await {
                *host = Some(Arc::new(again));
            }
            return Err(error.to_string());
        }
    }
    drop(host);
    Ok(info(&services, &state.home).await)
}
