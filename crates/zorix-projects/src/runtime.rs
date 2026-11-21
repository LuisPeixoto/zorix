//! Putting the pieces together for a program that owns the data folder: the command line, `zorix serve` and the desktop app.

use std::path::Path;
use std::sync::Arc;

use zorix_agents::ModelResolver;
use zorix_core::{ExecutionEngine, Result, SystemClock, TenantId, ToolRegistry};
use zorix_hub::Hub;
use zorix_runtime::Engine;
use zorix_store::{SqliteProjectStore, database_path};
use zorix_workflows::launcher::HubLauncher;

use crate::links::HubPullReader;
use crate::service::ProjectService;
use crate::tools::{ProjectTicker, project_tools};

/// What a program needs to use projects: the service, with its tools registered for runs that were granted them.
pub fn open(
    home: &Path,
    tenant: TenantId,
    hub: Arc<Hub>,
    engine: Engine,
    resolver: Arc<dyn ModelResolver>,
) -> Result<Arc<ProjectService>> {
    let store = Arc::new(SqliteProjectStore::open(&database_path(home))?);
    let executions: Arc<dyn ExecutionEngine> = Arc::new(engine);
    let launcher = Arc::new(HubLauncher::new(hub.clone(), executions, resolver, tenant.clone()));
    let pulls = Arc::new(HubPullReader::new(hub.clone(), tenant.clone()));
    let service = Arc::new(ProjectService::new(store, launcher, tenant, Arc::new(SystemClock)).with_pulls(pulls));
    for tool in project_tools(Arc::downgrade(&service)) {
        hub.registry.replace(tool)?;
    }
    Ok(service)
}

/// The ticker that keeps tasks moving, for the workflow host.
pub fn ticker(service: &Arc<ProjectService>) -> Arc<ProjectTicker> {
    Arc::new(ProjectTicker::new(service.clone()))
}
