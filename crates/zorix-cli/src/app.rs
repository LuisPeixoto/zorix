//! Opening the engine and the store.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use zorix_core::{ModelGateway, StepPhase};
use zorix_memory::MemoryStack;
use zorix_runtime::{Engine, EngineConfig, InMemoryToolRegistry};
use zorix_sim::{DemoModel, ScriptedModel};
use zorix_store::{DataDirLock, SqliteCalibration, SqliteMemoryStore, SqliteRepository, database_path};
use zorix_tools::{BuiltinConfig, register_builtin};

use zorix_providers::{ProvidersConfig, Router, SecretStore, build_router, config_path, secrets_with};

use crate::cli::{ModelArgs, ModelChoice};

/// A failure the command line reports to a person, as one sentence.
#[derive(Debug)]
pub struct CliError(pub String);

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<zorix_core::Error> for CliError {
    fn from(error: zorix_core::Error) -> Self {
        CliError(error.to_string())
    }
}

impl From<zorix_store::LockError> for CliError {
    fn from(error: zorix_store::LockError) -> Self {
        CliError(error.to_string())
    }
}

impl From<std::io::Error> for CliError {
    fn from(error: std::io::Error) -> Self {
        CliError(error.to_string())
    }
}

impl From<serde_json::Error> for CliError {
    fn from(error: serde_json::Error) -> Self {
        CliError(error.to_string())
    }
}

/// The data folder: the `--home` option (or `ZORIX_HOME`), else the platform default.
pub fn home_dir(option: Option<PathBuf>) -> PathBuf {
    option.unwrap_or_else(zorix_store::data_dir)
}

/// An engine, and the lock that keeps other ZORIX programs away from the data folder while it works.
pub struct App {
    /// The engine.
    pub engine: Engine,
    /// Memory: tools, context sources and the recorder.
    pub stack: Arc<MemoryStack>,
    /// The model reference to record for a new execution.
    pub model: String,
    _lock: DataDirLock,
}

/// The built-in tools in a fresh registry.
pub fn builtin_registry(home: &Path) -> Result<Arc<InMemoryToolRegistry>, CliError> {
    let registry = Arc::new(InMemoryToolRegistry::new());
    register_builtin(&*registry, &BuiltinConfig { notes_file: home.join("notes.json") })?;
    Ok(registry)
}

/// The providers configuration of the data folder. A missing file is an empty one.
pub fn load_config(home: &Path) -> Result<ProvidersConfig, CliError> {
    ProvidersConfig::load(&config_path(home)).map_err(CliError)
}

/// Where keys are kept. `ZORIX_SECRETS=env` leaves the keychain out (servers, CI, tests).
pub fn secrets(config: &ProvidersConfig) -> Arc<dyn SecretStore> {
    secrets_with(config, std::env::var("ZORIX_SECRETS").map(|v| v != "env").unwrap_or(true))
}

fn sim(choice: Option<&ModelChoice>) -> Result<Arc<dyn ModelGateway>, CliError> {
    Ok(match choice {
        Some(ModelChoice::Script(path)) => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| CliError(format!("Cannot read the script {}: {e}", path.display())))?;
            Arc::new(ScriptedModel::from_json(&text).map_err(CliError)?)
        }
        _ => Arc::new(DemoModel::new()),
    })
}

/// The router for a command: providers from `providers.toml`, keys from the keychain, the simulated models.
pub fn open_router(home: &Path, args: &ModelArgs) -> Result<Arc<Router>, CliError> {
    let config = load_config(home)?;
    let store = secrets(&config);
    let router = build_router(&config, store, Some(sim(args.model.as_ref())?), args.local_only).map_err(CliError)?;
    Ok(Arc::new(router))
}

/// Lock the data folder, open the store and start an engine on it.
pub fn open_engine(home: &Path, args: &ModelArgs) -> Result<App, CliError> {
    let router = open_router(home, args)?;
    let model = match &args.model {
        Some(choice) => choice.label(),
        // Nothing asked for: the default when it exists, otherwise the simulated model, as in 0.1.
        None if router.resolve("default", StepPhase::Start).is_ok() => "default".to_string(),
        None => "demo".to_string(),
    };
    let lock = DataDirLock::acquire(home)?;
    let repository = Arc::new(SqliteRepository::open(&database_path(home))?);
    let config = EngineConfig {
        // The lock proves nobody else uses the store, so what a crashed run left behind belongs to nobody.
        sole_owner: true,
        ..EngineConfig::default()
    };
    let database = database_path(home);
    let store = Arc::new(SqliteMemoryStore::open(&database)?);
    let gateway: Arc<dyn ModelGateway> = router;
    let stack = Arc::new(MemoryStack::new(store, repository.clone(), Some(gateway.clone())));
    let registry = builtin_registry(home)?;
    stack.register_tools(&*registry)?;
    let calibration = Arc::new(SqliteCalibration::open(&database)?);
    let engine = stack
        .install(Engine::builder(repository, gateway).tools(registry).calibration(calibration.clone()).config(config))
        .build();
    // What was recorded before this version, or while a queue was full, is caught up in the background.
    let background = stack.clone();
    tokio::spawn(async move {
        let _ = background.backfill_operations().await;
    });
    tokio::task::spawn_blocking(move || {
        let _ = calibration.backfill();
    });
    Ok(App { engine, stack, model, _lock: lock })
}

impl App {
    /// Stop the engine, then write the records of anything that finished while the command ran.
    pub async fn shutdown(&self) {
        self.engine.shutdown().await;
        let _ = self.stack.backfill_operations().await;
    }
}

/// Open the store for reading only. Takes no lock, so it works while another ZORIX program is running.
pub fn open_reader(home: &Path) -> Result<SqliteRepository, CliError> {
    Ok(SqliteRepository::open(&database_path(home))?)
}
