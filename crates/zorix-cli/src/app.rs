//! Opening the engine and the store.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use zorix_core::{Grants, ModelGateway, StepPhase, TenantId};
use zorix_hub::Hub;
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

impl From<zorix_hub::HubError> for CliError {
    fn from(error: zorix_hub::HubError) -> Self {
        CliError(error.0)
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
    /// Connections: MCP servers, integrations and the sandbox.
    pub hub: Arc<Hub>,
    /// The model reference to record for a new execution.
    pub model: String,
    /// Turns model names into candidates, for agents.
    pub router: Arc<Router>,
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
    // Debug builds only (so tests can keep secrets across processes without touching a real keychain). A release build has
    // no such switch: secrets go to the keychain.
    #[cfg(debug_assertions)]
    if let Ok(path) = std::env::var("ZORIX_TEST_SECRETS_FILE") {
        return Arc::new(test_store::FileStore(std::path::PathBuf::from(path)));
    }
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
pub fn open_engine(home: &Path, tenant: &TenantId, args: &ModelArgs) -> Result<App, CliError> {
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
    let gateway: Arc<dyn ModelGateway> = router.clone();
    let stack = Arc::new(MemoryStack::new(store, repository.clone(), Some(gateway.clone())));
    let registry = builtin_registry(home)?;
    stack.register_tools(&*registry)?;
    let hub = Arc::new(Hub::open(home, tenant.clone(), secrets(&load_config(home)?), registry.clone())?);
    hub.register_native_tools()?;
    let calibration = Arc::new(SqliteCalibration::open(&database)?);
    let engine = stack
        .install(
            Engine::builder(repository, gateway)
                .tools(registry)
                .delegation(hub.planner(router.clone()))
                .calibration(calibration.clone())
                .config(config),
        )
        .build();
    // What was recorded before this version, or while a queue was full, is caught up in the background.
    let background = stack.clone();
    tokio::spawn(async move {
        let _ = background.backfill_operations().await;
    });
    tokio::task::spawn_blocking(move || {
        let _ = calibration.backfill();
    });
    Ok(App { engine, stack, hub, model, router, _lock: lock })
}

impl App {
    /// Stop the engine, then write the records of anything that finished while the command ran.
    pub async fn shutdown(&self) {
        self.hub.manager.shutdown().await;
        self.engine.shutdown().await;
        let _ = self.stack.backfill_operations().await;
    }
}

/// Open the store for reading only. Takes no lock, so it works while another ZORIX program is running.
pub fn open_reader(home: &Path) -> Result<SqliteRepository, CliError> {
    Ok(SqliteRepository::open(&database_path(home))?)
}

impl App {
    /// Start the MCP servers a set of grants names and tell the person about any that is not ready. A run that needs a server
    /// that is down will say so when it calls the tool; this only makes sure the usual case is ready first.
    pub async fn start_servers(&self, grants: &Grants) {
        match self.hub.start_for(grants, std::time::Duration::from_secs(30)).await {
            Ok(trouble) => {
                for (name, why) in trouble {
                    eprintln!("zorix: the MCP server {name} is not ready: {why}");
                }
            }
            Err(error) => eprintln!("zorix: could not start MCP servers: {error}"),
        }
    }
}

#[cfg(debug_assertions)]
mod test_store {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use zorix_core::Secret;
    use zorix_providers::{SecretError, SecretStore};

    /// Secrets in a JSON file, for tests in debug builds.
    pub struct FileStore(pub PathBuf);

    impl FileStore {
        fn read(&self) -> BTreeMap<String, String> {
            std::fs::read_to_string(&self.0).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
        }

        fn write(&self, map: &BTreeMap<String, String>) -> Result<(), SecretError> {
            std::fs::write(&self.0, serde_json::to_string(map).unwrap_or_default())
                .map_err(|e| SecretError::Failed(e.to_string()))
        }
    }

    impl SecretStore for FileStore {
        fn get(&self, name: &str) -> Result<Option<Secret>, SecretError> {
            Ok(self.read().get(name).map(|v| Secret::new(v.clone())))
        }

        fn set(&self, name: &str, value: &Secret) -> Result<(), SecretError> {
            let mut map = self.read();
            map.insert(name.to_string(), value.expose().to_string());
            self.write(&map)
        }

        fn remove(&self, name: &str) -> Result<bool, SecretError> {
            let mut map = self.read();
            let had = map.remove(name).is_some();
            self.write(&map)?;
            Ok(had)
        }

        fn describe(&self) -> String {
            "a test file".into()
        }
    }
}
