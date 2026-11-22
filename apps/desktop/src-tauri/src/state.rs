//! What the app holds for as long as it runs: the engine, and the lock on the data folder.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;
use zorix_core::{ExecutionId, TenantId};
use zorix_hub::Hub;
use zorix_memory::MemoryStack;
use zorix_providers::{ProvidersConfig, Router, SecretStore, build_router, config_path, secrets_with};
use zorix_runtime::{Engine, EngineConfig, InMemoryToolRegistry};
use zorix_sim::DemoModel;
use zorix_store::{DataDirLock, SqliteCalibration, SqliteMemoryStore, SqliteRepository, database_path};
use zorix_tools::{BuiltinConfig, register_builtin};

/// The running engine.
pub struct Services {
    /// The engine.
    pub engine: Engine,
    /// Whose data the app shows.
    pub tenant: TenantId,
    /// Routes model references to providers. Keys are read when a call is made, so a new key works at once.
    pub router: Arc<Router>,
    /// Where keys are kept.
    pub secrets: Arc<dyn SecretStore>,
    /// The providers configuration read at start.
    pub config: ProvidersConfig,
    /// Memory: tools, context sources, the recorder and the indexer.
    pub memory: Arc<MemoryStack>,
    /// Where memory is kept.
    pub memory_store: Arc<SqliteMemoryStore>,
    /// Connections: MCP servers, integrations and the sandbox.
    pub hub: Arc<Hub>,
    /// Workflows: their store and the engine that runs them.
    pub workflows: zorix_workflows::Workflows,
    /// Projects and tasks: the rules, and the tools a manager and a task's run use.
    pub projects: Arc<zorix_projects::ProjectService>,
    /// Workers, their levels and budgets, and the manager's week.
    pub workforce: Arc<zorix_workforce::WorkforceService>,
    /// What starts workflows by itself while the app is open: schedules, events and (when on) webhooks.
    pub host: tokio::sync::Mutex<Option<Arc<zorix_workflows::WorkflowHost>>>,
    /// Sign-ins that are waiting for the browser, so they can be cancelled.
    pub logins: Mutex<HashMap<String, CancellationToken>>,
    /// Knowledge syncs that are running, so they can be cancelled.
    pub syncs: Mutex<HashMap<zorix_core::KnowledgeRootId, CancellationToken>>,
    /// The job that embeds everything again, when one is running.
    pub reembed_job: Mutex<Option<CancellationToken>>,
    /// Evaluations that are running, by agent, so they can be cancelled and not started twice.
    pub evals: Mutex<HashMap<String, CancellationToken>>,
    _lock: DataDirLock,
}

/// The app's state, managed by Tauri.
pub struct AppState {
    /// The data folder.
    pub home: PathBuf,
    /// The engine, or why it could not start (for example another ZORIX program holds the data folder).
    pub services: Result<Arc<Services>, String>,
    watchers: Mutex<HashMap<ExecutionId, CancellationToken>>,
    live: Mutex<HashMap<ExecutionId, CancellationToken>>,
}

impl AppState {
    /// Open the store, start the engine and continue what a crash or a closed window left half done.
    pub async fn start() -> Self {
        let home = zorix_store::data_dir();
        let services = open(&home).await;
        if let Err(error) = &services {
            tracing::error!(%error, "the engine did not start");
        }
        Self { home, services, watchers: Mutex::new(HashMap::new()), live: Mutex::new(HashMap::new()) }
    }

    /// The engine, or the reason there is none.
    pub fn services(&self) -> Result<&Services, String> {
        self.services.as_deref().map_err(Clone::clone)
    }

    /// The same, for work that goes on after the command returns.
    pub fn services_arc(&self) -> Result<Arc<Services>, String> {
        self.services.clone()
    }

    /// Register a watcher for an execution, replacing (and stopping) an earlier one.
    pub fn watch(&self, id: ExecutionId) -> CancellationToken {
        let token = CancellationToken::new();
        if let Some(old) = self.watchers.lock().insert(id, token.clone()) {
            old.cancel();
        }
        token
    }

    /// Register a watcher of an execution's live text, replacing (and stopping) an earlier one.
    pub fn watch_live(&self, id: ExecutionId) -> CancellationToken {
        let token = CancellationToken::new();
        if let Some(old) = self.live.lock().insert(id, token.clone()) {
            old.cancel();
        }
        token
    }

    /// Stop watching an execution's live text.
    pub fn unwatch_live(&self, id: ExecutionId) {
        if let Some(token) = self.live.lock().remove(&id) {
            token.cancel();
        }
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
        for (_, token) in self.live.lock().drain() {
            token.cancel();
        }
        if let Ok(services) = &self.services {
            for (_, token) in services.syncs.lock().drain() {
                token.cancel();
            }
            if let Some(token) = services.reembed_job.lock().take() {
                token.cancel();
            }
            for (_, token) in services.evals.lock().drain() {
                token.cancel();
            }
            if let Some(host) = services.host.lock().await.take() {
                host.shutdown().await;
            }
            services.hub.manager.shutdown().await;
            services.engine.shutdown().await;
            let _ = services.memory.backfill_operations().await;
        }
    }
}

async fn open(home: &Path) -> Result<Arc<Services>, String> {
    use zorix_core::ExecutionEngine;
    let lock = DataDirLock::acquire(home).map_err(|e| e.to_string())?;
    let repository = Arc::new(SqliteRepository::open(&database_path(home)).map_err(|e| e.to_string())?);
    let registry = Arc::new(InMemoryToolRegistry::new());
    register_builtin(&*registry, &BuiltinConfig { notes_file: home.join("notes.json") }).map_err(|e| e.to_string())?;
    let config = EngineConfig { sole_owner: true, ..EngineConfig::default() };
    let providers = ProvidersConfig::load(&config_path(home))?;
    let keychain = std::env::var("ZORIX_SECRETS").map(|v| v != "env").unwrap_or(true);
    let secrets = secrets_with(&providers, keychain);
    let router = Arc::new(build_router(&providers, secrets.clone(), Some(Arc::new(DemoModel::new())), false)?);
    let database = database_path(home);
    let memory_store = Arc::new(SqliteMemoryStore::open(&database).map_err(|e| e.to_string())?);
    let gateway: Arc<dyn zorix_core::ModelGateway> = router.clone();
    let memory = Arc::new(MemoryStack::new(memory_store.clone(), repository.clone(), Some(gateway.clone())));
    memory.register_tools(&*registry).map_err(|e| e.to_string())?;
    let hub =
        Arc::new(Hub::open(home, TenantId::local(), secrets.clone(), registry.clone()).map_err(|e| e.to_string())?);
    hub.register_native_tools().map_err(|e| e.to_string())?;
    let calibration = Arc::new(SqliteCalibration::open(&database).map_err(|e| e.to_string())?);
    let engine = memory
        .install(
            Engine::builder(repository, gateway)
                .tools(registry)
                .delegation(hub.planner(router.clone()))
                .calibration(calibration.clone())
                .config(config),
        )
        .build();
    {
        // What was recorded before this version, or while a queue was full, is caught up in the background.
        let background = memory.clone();
        tokio::spawn(async move {
            let _ = background.backfill_operations().await;
        });
        tokio::task::spawn_blocking(move || {
            let _ = calibration.backfill();
        });
    }
    match engine.recover().await {
        Ok(report) if !report.requeued.is_empty() || !report.needs_decision.is_empty() => {
            tracing::info!(requeued = report.requeued.len(), needs_decision = report.needs_decision.len(), "recovered");
        }
        Ok(_) => {}
        Err(error) => tracing::warn!(%error, "recovery failed"),
    }
    {
        // Servers that were left on start in the background; each shows its state as it gets there.
        let hub = hub.clone();
        tokio::spawn(async move {
            let _ = hub.manager.start_all().await;
        });
    }
    let workflows =
        zorix_workflows::Workflows::open(home, TenantId::local(), hub.clone(), engine.clone(), router.clone())
            .map_err(|e| e.to_string())?;
    let settings = crate::settings::Settings::load(home);
    let webhook = settings.listen().unwrap_or_else(|error| {
        tracing::warn!(%error, "webhooks stay off");
        None
    });
    let projects = zorix_projects::runtime::open(home, TenantId::local(), hub.clone(), engine.clone(), router.clone())
        .map_err(|e| e.to_string())?;
    let workforce =
        zorix_workforce::runtime::open(home, TenantId::local(), hub.clone(), engine.clone(), router.clone(), &projects)
            .map_err(|e| e.to_string())?;
    let tickers: Vec<Arc<dyn zorix_workflows::host::Ticker>> =
        vec![zorix_projects::runtime::ticker(&projects), zorix_workforce::runtime::ticker(&workforce)];
    let host = match workflows.start_host_with(secrets.clone(), webhook, tickers).await {
        Ok(host) => Some(Arc::new(host)),
        Err(error) => {
            tracing::warn!(%error, "the workflow host did not start; schedules and webhooks are off");
            None
        }
    };
    Ok(Arc::new(Services {
        engine,
        hub,
        workflows,
        projects,
        workforce,
        host: tokio::sync::Mutex::new(host),
        logins: Mutex::new(HashMap::new()),
        tenant: TenantId::local(),
        router,
        secrets,
        config: providers,
        memory,
        memory_store,
        syncs: Mutex::new(HashMap::new()),
        reembed_job: Mutex::new(None),
        evals: Mutex::new(HashMap::new()),
        _lock: lock,
    }))
}
