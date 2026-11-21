//! Putting the pieces together for a program that owns the data folder: the command line's `zorix serve` and the desktop app.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use zorix_agents::ModelResolver;
use zorix_core::{Error, ExecutionEngine, Result, SystemClock, TenantId};
use zorix_hub::Hub;
use zorix_providers::SecretStore;
use zorix_runtime::Engine;
use zorix_store::{SqliteWorkflowStore, database_path};

use crate::engine::WorkflowEngine;
use crate::host::{EngineFeed, HostConfig, WorkflowHost};
use crate::launcher::HubLauncher;
use crate::service::WorkflowService;

/// The default place webhooks arrive when they are switched on: this computer only.
pub const DEFAULT_WEBHOOK_ADDRESS: &str = "127.0.0.1:7788";

/// What a program needs to run workflows.
#[derive(Clone)]
pub struct Workflows {
    /// Where workflows are kept.
    pub store: Arc<SqliteWorkflowStore>,
    /// Runs them.
    pub engine: Arc<WorkflowEngine>,
    /// What the screens and commands ask that needs no running engine.
    pub service: WorkflowService,
    executions: Arc<dyn ExecutionEngine>,
}

impl Workflows {
    /// Open the workflow store of a data folder and build an engine over the execution engine and the hub.
    pub fn open(
        home: &Path,
        tenant: TenantId,
        hub: Arc<Hub>,
        executions: Engine,
        resolver: Arc<dyn ModelResolver>,
    ) -> Result<Self> {
        let store = Arc::new(SqliteWorkflowStore::open(&database_path(home))?);
        let executions: Arc<dyn ExecutionEngine> = Arc::new(executions);
        let launcher = Arc::new(HubLauncher::new(hub.clone(), executions.clone(), resolver, tenant.clone()));
        let engine = Arc::new(WorkflowEngine::new(store.clone(), launcher, Arc::new(SystemClock), tenant.clone()));
        let service = WorkflowService::new(store.clone(), tenant, hub, Arc::new(SystemClock));
        Ok(Self { store, engine, service, executions })
    }

    /// Start the host: schedules, events, and webhooks when an address is given.
    pub async fn start_host(&self, secrets: Arc<dyn SecretStore>, webhook: Option<SocketAddr>) -> Result<WorkflowHost> {
        self.start_host_with(secrets, webhook, Vec::new()).await
    }

    /// Start the host with more work done at the end of every beat (such as keeping tasks moving).
    pub async fn start_host_with(
        &self,
        secrets: Arc<dyn SecretStore>,
        webhook: Option<SocketAddr>,
        tickers: Vec<Arc<dyn crate::host::Ticker>>,
    ) -> Result<WorkflowHost> {
        let feed = Arc::new(EngineFeed::new(self.executions.clone(), self.engine.tenant().clone()));
        WorkflowHost::start(
            self.engine.clone(),
            feed,
            secrets,
            Arc::new(SystemClock),
            HostConfig { webhook, tickers, ..HostConfig::default() },
        )
        .await
    }
}

/// Read an address a person typed.
pub fn parse_address(text: &str) -> Result<SocketAddr> {
    text.parse().map_err(|_| Error::InvalidInput(format!("“{text}” is not an address like 127.0.0.1:7788.")))
}
