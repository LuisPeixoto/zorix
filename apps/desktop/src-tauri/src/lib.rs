//! Tauri shell: starts the engine and exposes it to the interface as commands. It holds no rules of its own.

mod commands;
mod integration_commands;
mod knowledge_commands;
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
            if let Ok(services) = app.state::<AppState>().services_arc() {
                crate::integration_commands::forward_status(app.handle().clone(), services.hub.manager.clone());
            }
            // Folders marked "check at start" are brought up to date in the background.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move { crate::knowledge_commands::sync_at_start(handle).await });
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
            integration_commands::mcp_list,
            integration_commands::mcp_server,
            integration_commands::mcp_catalog,
            integration_commands::mcp_add,
            integration_commands::mcp_set_enabled,
            integration_commands::mcp_remove,
            integration_commands::mcp_restart,
            integration_commands::mcp_refresh,
            integration_commands::mcp_review,
            integration_commands::mcp_approve_all,
            integration_commands::mcp_trust,
            integration_commands::mcp_login,
            integration_commands::mcp_login_cancel,
            integration_commands::mcp_logout,
            integration_commands::mcp_import_preview,
            integration_commands::mcp_import_apply,
            integration_commands::connections_list,
            integration_commands::connection_add,
            integration_commands::connection_test,
            integration_commands::connection_remove,
            integration_commands::sandbox_status,
            integration_commands::sandbox_install_python,
            memory_commands::list_memory,
            memory_commands::add_memory,
            memory_commands::update_memory,
            memory_commands::memory_usage,
            memory_commands::forget_memory,
            memory_commands::run_context,
            knowledge_commands::list_roots,
            knowledge_commands::add_root,
            knowledge_commands::sync_root,
            knowledge_commands::cancel_sync,
            knowledge_commands::remove_root,
            knowledge_commands::root_files,
            knowledge_commands::try_query,
            knowledge_commands::embeddings_info,
            knowledge_commands::embedding_models,
            knowledge_commands::set_embeddings,
            knowledge_commands::embeddings_off,
            knowledge_commands::test_embeddings,
            knowledge_commands::reembed,
            knowledge_commands::reembedding,
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
