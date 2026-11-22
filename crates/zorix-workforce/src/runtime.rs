//! Putting the pieces together for a program that owns the data folder: the command line, `zorix serve` and the desktop app.

use std::path::Path;
use std::sync::Arc;

use zorix_agents::ModelResolver;
use zorix_core::{AgentStore, ExecutionEngine, Result, SystemClock, TenantId, ToolRegistry};
use zorix_hub::Hub;
use zorix_projects::ProjectService;
use zorix_runtime::Engine;
use zorix_store::{SqliteOrgStore, SqliteProjectStore, database_path};
use zorix_workflows::launcher::HubLauncher;

use crate::service::WorkforceService;
use crate::tools::{WorkforceTicker, org_tools};

/// What a program needs to use the workforce: the service, with `org.team` registered and the project service told to shape the
/// runs it starts.
pub fn open(
    home: &Path,
    tenant: TenantId,
    hub: Arc<Hub>,
    engine: Engine,
    resolver: Arc<dyn ModelResolver>,
    projects: &Arc<ProjectService>,
) -> Result<Arc<WorkforceService>> {
    let path = database_path(home);
    let org = Arc::new(SqliteOrgStore::open(&path)?);
    let project_store = Arc::new(SqliteProjectStore::open(&path)?);
    let agents: Arc<dyn AgentStore> = hub.agents.clone();
    let executions: Arc<dyn ExecutionEngine> = Arc::new(engine);
    let launcher = Arc::new(HubLauncher::new(hub.clone(), executions.clone(), resolver, tenant.clone()));
    let service = Arc::new(WorkforceService::new(
        org,
        project_store,
        agents,
        executions,
        launcher,
        tenant,
        Arc::new(SystemClock),
    ));
    for tool in org_tools(Arc::downgrade(&service)) {
        hub.registry.replace(tool)?;
    }
    projects.set_shaper(Some(service.clone()));
    Ok(service)
}

/// The ticker that keeps the workforce moving, for the workflow host.
pub fn ticker(service: &Arc<WorkforceService>) -> Arc<WorkforceTicker> {
    Arc::new(WorkforceTicker::new(service.clone()))
}
