//! Opening the engine and the store.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use zorix_core::ModelGateway;
use zorix_runtime::{Engine, EngineConfig, InMemoryToolRegistry};
use zorix_sim::{DemoModel, ScriptedModel};
use zorix_store::{DataDirLock, SqliteRepository, database_path};
use zorix_tools::{BuiltinConfig, register_builtin};

use crate::cli::ModelChoice;

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
    _lock: DataDirLock,
}

/// The built-in tools in a fresh registry.
pub fn builtin_registry(home: &Path) -> Result<Arc<InMemoryToolRegistry>, CliError> {
    let registry = Arc::new(InMemoryToolRegistry::new());
    register_builtin(&*registry, &BuiltinConfig { notes_file: home.join("notes.json") })?;
    Ok(registry)
}

fn gateway(choice: &ModelChoice) -> Result<Arc<dyn ModelGateway>, CliError> {
    Ok(match choice {
        ModelChoice::Demo => Arc::new(DemoModel::new()),
        ModelChoice::Script(path) => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| CliError(format!("Cannot read the script {}: {e}", path.display())))?;
            Arc::new(ScriptedModel::from_json(&text).map_err(CliError)?)
        }
    })
}

/// Lock the data folder, open the store and start an engine on it.
pub fn open_engine(home: &Path, model: &ModelChoice) -> Result<App, CliError> {
    let lock = DataDirLock::acquire(home)?;
    let repository = Arc::new(SqliteRepository::open(&database_path(home))?);
    let config = EngineConfig {
        // The lock proves nobody else uses the store, so what a crashed run left behind belongs to nobody.
        sole_owner: true,
        ..EngineConfig::default()
    };
    let engine = Engine::builder(repository, gateway(model)?).tools(builtin_registry(home)?).config(config).build();
    Ok(App { engine, _lock: lock })
}

/// Open the store for reading only. Takes no lock, so it works while another ZORIX program is running.
pub fn open_reader(home: &Path) -> Result<SqliteRepository, CliError> {
    Ok(SqliteRepository::open(&database_path(home))?)
}
