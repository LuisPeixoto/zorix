//! What the app holds for as long as it runs: the engine, and the lock on the data folder.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;
use zorix_core::{ExecutionId, TenantId};
use zorix_runtime::{Engine, EngineConfig, InMemoryToolRegistry};
use zorix_sim::DemoModel;
use zorix_store::{DataDirLock, SqliteRepository, database_path};
use zorix_tools::{BuiltinConfig, register_builtin};

/// The running engine.
pub struct Services {
    /// The engine.
    pub engine: Engine,
    /// Whose data the app shows.
    pub tenant: TenantId,
    _lock: DataDirLock,
}

/// The app's state, managed by Tauri.
pub struct AppState {
    /// The data folder.
    pub home: PathBuf,
    /// The engine, or why it could not start (for example another ZORIX program holds the data folder).
    pub services: Result<Services, String>,
    watchers: Mutex<HashMap<ExecutionId, CancellationToken>>,
}

impl AppState {
    /// Open the store, start the engine and continue what a crash or a closed window left half done.
    pub async fn start() -> Self {
        let home = zorix_store::data_dir();
        let services = open(&home).await;
        if let Err(error) = &services {
            tracing::error!(%error, "the engine did not start");
        }
        Self { home, services, watchers: Mutex::new(HashMap::new()) }
    }

    /// The engine, or the reason there is none.
    pub fn services(&self) -> Result<&Services, String> {
        self.services.as_ref().map_err(Clone::clone)
    }

    /// Register a watcher for an execution, replacing (and stopping) an earlier one.
    pub fn watch(&self, id: ExecutionId) -> CancellationToken {
        let token = CancellationToken::new();
        if let Some(old) = self.watchers.lock().insert(id, token.clone()) {
            old.cancel();
        }
        token
    }

    /// Stop watching an execution.
    pub fn unwatch(&self, id: ExecutionId) {
        if let Some(token) = self.watchers.lock().remove(&id) {
            token.cancel();
        }
    }

    /// Stop the engine gracefully: work in progress is left in its last checkpoint and continues next time.
    pub async fn shutdown(&self) {
        for (_, token) in self.watchers.lock().drain() {
            token.cancel();
        }
        if let Ok(services) = &self.services {
            services.engine.shutdown().await;
        }
    }
}

async fn open(home: &Path) -> Result<Services, String> {
    use zorix_core::ExecutionEngine;
    let lock = DataDirLock::acquire(home).map_err(|e| e.to_string())?;
    let repository = Arc::new(SqliteRepository::open(&database_path(home)).map_err(|e| e.to_string())?);
    let registry = Arc::new(InMemoryToolRegistry::new());
    register_builtin(&*registry, &BuiltinConfig { notes_file: home.join("notes.json") }).map_err(|e| e.to_string())?;
    let config = EngineConfig { sole_owner: true, ..EngineConfig::default() };
    let engine = Engine::builder(repository, Arc::new(DemoModel::new())).tools(registry).config(config).build();
    match engine.recover().await {
        Ok(report) if !report.requeued.is_empty() || !report.needs_decision.is_empty() => {
            tracing::info!(requeued = report.requeued.len(), needs_decision = report.needs_decision.len(), "recovered");
        }
        Ok(_) => {}
        Err(error) => tracing::warn!(%error, "recovery failed"),
    }
    Ok(Services { engine, tenant: TenantId::local(), _lock: lock })
}
