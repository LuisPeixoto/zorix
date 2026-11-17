//! Tauri shell: starts the engine and exposes it to the interface as commands. It holds no rules of its own.

mod commands;
mod memory_commands;
mod state;

use tauri::{Manager, RunEvent};

use crate::state::AppState;

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("ZORIX_LOG").unwrap_or_else(|_| "info".into()))
        .init();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // The engine starts workers on the async runtime, so it must be created inside it.
            let state = tauri::async_runtime::block_on(AppState::start());
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::list_tools,
            commands::list_runs,
            commands::run_detail,
            commands::start_run,
            commands::watch_run,
            commands::stop_watch,
            commands::decide_run,
            commands::cancel_run,
            commands::pause_run,
            commands::resume_run,
            commands::list_providers,
            commands::set_provider_key,
            commands::remove_provider_key,
            commands::test_provider,
            commands::list_models,
            memory_commands::list_memory,
            memory_commands::add_memory,
            memory_commands::update_memory,
            memory_commands::memory_usage,
            memory_commands::forget_memory,
            memory_commands::run_context,
        ])
        .build(tauri::generate_context!())
        .expect("error while building the ZORIX app");

    app.run(|handle, event| {
        if let RunEvent::ExitRequested { .. } = event
            && let Some(state) = handle.try_state::<AppState>()
        {
            // Leave work in its last checkpoint; it continues the next time ZORIX starts.
            tauri::async_runtime::block_on(state.shutdown());
        }
    });
}
