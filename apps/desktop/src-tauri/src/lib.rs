//! Tauri shell: starts the engine and exposes it to the interface as commands. It holds no rules of its own.

mod agent_commands;
mod commands;
mod integration_commands;
mod knowledge_commands;
mod memory_commands;
mod project_commands;
mod settings;
mod state;
mod workflow_commands;
mod workforce_commands;

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
            project_commands::projects_overview,
            project_commands::project_create,
            project_commands::project_update,
            project_commands::project_remove,
            project_commands::project_board,
            project_commands::project_ask,
            project_commands::project_history,
            project_commands::task_add,
            project_commands::task_update,
            project_commands::task_move,
            project_commands::task_restart,
            project_commands::task_remove,
            project_commands::task_detail,
            project_commands::task_comment,
            project_commands::task_link,
            project_commands::task_deliver,
            project_commands::tasks_needing_you,
            project_commands::tasks_index,
            workforce_commands::workforce_overview,
            workforce_commands::worker_detail,
            workforce_commands::worker_hire,
            workforce_commands::worker_edit,
            workforce_commands::worker_fire,
            workforce_commands::workforce_settings_save,
            workforce_commands::workforce_activity,
            workforce_commands::workforce_notices,
            workforce_commands::workforce_mark_seen,
            workforce_commands::worker_plan,
            workforce_commands::worker_report,
            workflow_commands::workflow_host,
            workflow_commands::workflow_settings_set,
            workflow_commands::workflows_overview,
            workflow_commands::workflow_detail,
            workflow_commands::workflow_create,
            workflow_commands::workflow_save,
            workflow_commands::workflow_publish,
            workflow_commands::workflow_rollback,
            workflow_commands::workflow_set_enabled,
            workflow_commands::workflow_remove,
            workflow_commands::workflow_export,
            workflow_commands::workflow_import,
            workflow_commands::workflow_templates,
            workflow_commands::workflow_add_template,
            workflow_commands::workflow_diff,
            workflow_commands::workflow_outline,
            workflow_commands::workflow_start,
            workflow_commands::workflow_runs,
            workflow_commands::workflow_run,
            workflow_commands::workflow_cancel,
            workflow_commands::workflow_waits,
            workflow_commands::workflow_decide,
            workflow_commands::workflow_step_history,
            workflow_commands::workflow_webhook,
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
            agent_commands::agents_list,
            agent_commands::agent_get,
            agent_commands::agent_save,
            agent_commands::agent_import,
            agent_commands::agent_export,
            agent_commands::agent_versions,
            agent_commands::agent_diff,
            agent_commands::agent_rollback,
            agent_commands::agent_archive,
            agent_commands::agent_remove,
            agent_commands::agent_templates,
            agent_commands::agent_add_template,
            agent_commands::skills_list,
            agent_commands::skill_import,
            agent_commands::skill_remove,
            agent_commands::skill_file,
            agent_commands::eval_cases,
            agent_commands::eval_put_case,
            agent_commands::eval_remove_case,
            agent_commands::eval_runs,
            agent_commands::eval_running,
            agent_commands::eval_start,
            agent_commands::eval_cancel,
            agent_commands::eval_compare,
            agent_commands::agent_ask,
            agent_commands::agent_threads,
            agent_commands::thread_runs,
            agent_commands::watch_live,
            agent_commands::stop_live,
            agent_commands::open_external,
            agent_commands::run_tree,
            agent_commands::tree_approvals,
            agent_commands::sub_run_counts,
            agent_commands::teams_list,
            agent_commands::team_save,
            agent_commands::team_remove,
            agent_commands::team_templates,
            agent_commands::team_add_template,
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
