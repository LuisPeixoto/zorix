//! The hub: ZORIX's connections to the outside, assembled once so the command line and the desktop app share them.
//!
//! It owns the integration store, the keychain, the tool registry, the MCP manager and the sandbox, and offers what a person
//! does with them: add and sign in to MCP servers, add and test connections, and turn flags into grants.

pub mod agents;
pub mod connections;
pub mod estimate;
pub mod grants;
pub mod ocr;
pub mod teams;
pub mod telemetry;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;
use zorix_core::{
    Clock, EnvValue, Grants, IntegrationStore, McpServer, McpTransport, NewMcpServer, Permission, SandboxFolder,
    Secret, ServerSandbox, SystemClock, TenantId, Timestamp,
};
use zorix_integrations::{Services, db_tools, discord_tools, forge_tools, http_tools, jira_tools, slack_tools};
use zorix_mcp::catalog::SecretNeeded;
use zorix_mcp::{
    BrowserOpener, LoginSummary, McpManager, OAuthClient, OAuthSettings, ServerState, Vault, catalog, env_secret_name,
    secret_base, token_secret_name,
};
use zorix_providers::SecretStore;
use zorix_runtime::InMemoryToolRegistry;
use zorix_sandbox::{Sandbox, sandbox_tools};
use zorix_store::{SqliteAgentStore, SqliteIntegrationStore, database_path};

/// Something a person can act on, as one sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubError(pub String);

impl std::fmt::Display for HubError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for HubError {}

impl From<zorix_core::Error> for HubError {
    fn from(error: zorix_core::Error) -> Self {
        HubError(error.to_string())
    }
}

impl From<zorix_mcp::McpError> for HubError {
    fn from(error: zorix_mcp::McpError) -> Self {
        HubError(error.to_string())
    }
}

impl From<zorix_sandbox::SandboxError> for HubError {
    fn from(error: zorix_sandbox::SandboxError) -> Self {
        HubError(error.to_string())
    }
}

/// The connections of one tenant.
pub struct Hub {
    tenant: TenantId,
    /// Where servers, tool pins and connections are kept.
    pub store: Arc<SqliteIntegrationStore>,
    /// Where their secrets are kept.
    pub secrets: Arc<dyn SecretStore>,
    /// The tools of the engine this hub feeds.
    pub registry: Arc<InMemoryToolRegistry>,
    /// Runs and supervises MCP servers.
    pub manager: McpManager,
    /// Runs code.
    pub sandbox: Arc<Sandbox>,
    /// Agents, their versions, skills and evaluations.
    pub agents: Arc<SqliteAgentStore>,
    /// The skill files on disk.
    pub skills: Arc<zorix_agents::SkillLibrary>,
    services: Arc<Services>,
}

impl Hub {
    /// Open the hub on a data folder. Nothing is started and no tool is registered yet.
    pub fn open(
        home: &Path,
        tenant: TenantId,
        secrets: Arc<dyn SecretStore>,
        registry: Arc<InMemoryToolRegistry>,
    ) -> Result<Self, HubError> {
        let store = Arc::new(SqliteIntegrationStore::open(&database_path(home))?);
        let dyn_store: Arc<dyn IntegrationStore> = store.clone();
        // A stdio server that names no folder starts in one of its own under the data folder, not wherever ZORIX was started from.
        let manager = McpManager::with(
            dyn_store.clone(),
            registry.clone(),
            secrets.clone(),
            tenant.clone(),
            Arc::new(zorix_mcp::DefaultConnector),
            Arc::new(zorix_core::SystemClock),
            zorix_mcp::ManagerOptions {
                work_dir: Some(home.join("mcp")),
                data_home: Some(home.to_path_buf()),
                ..zorix_mcp::ManagerOptions::default()
            },
        );
        let sandbox = Arc::new(Sandbox::new(home)?);
        let services = Arc::new(Services { store: dyn_store, secrets: secrets.clone() });
        let agents = Arc::new(SqliteAgentStore::open(&database_path(home))?);
        let skills = Arc::new(zorix_agents::SkillLibrary::new(home));
        Ok(Self { tenant, store, secrets, registry, manager, sandbox, agents, skills, services })
    }

    /// The tenant it works for.
    pub fn tenant(&self) -> &TenantId {
        &self.tenant
    }

    /// What the integration tools use.
    pub fn services(&self) -> Arc<Services> {
        self.services.clone()
    }

    /// Register the native tools: HTTP, Git hosts, Jira, Slack, Discord, databases and code. They are only offered to runs
    /// that were granted them.
    pub fn register_native_tools(&self) -> Result<(), HubError> {
        use zorix_core::ToolRegistry as _;
        let dyn_store: Arc<dyn IntegrationStore> = self.store.clone();
        let mut tools = http_tools(dyn_store, self.secrets.clone());
        tools.extend(forge_tools(self.services()));
        tools.extend(jira_tools(self.services()));
        tools.extend(slack_tools(self.services()));
        tools.extend(discord_tools(self.services()));
        tools.extend(db_tools(self.services()));
        tools.extend(sandbox_tools(self.sandbox.clone()));
        tools.extend(zorix_agents::skill_tools(self.agents.clone(), self.skills.clone(), self.sandbox.clone()));
        tools.push(Arc::new(zorix_core::DelegateTool::new()));
        for tool in tools {
            self.registry.register(tool)?;
        }
        Ok(())
    }

    /// The planner an engine needs for `agents.delegate`: it finds agents in this hub's store and resolves models with `resolver`.
    pub fn planner(&self, resolver: Arc<dyn zorix_agents::ModelResolver>) -> Arc<zorix_agents::AgentPlanner> {
        Arc::new(zorix_agents::AgentPlanner::new(self.agents.clone(), resolver))
    }

    /// Whether the skill tools are registered (tests).
    pub fn registry_has_skill_tools(&self) -> bool {
        use zorix_core::ToolRegistry as _;
        ["skill.load", "skill.read", "skill.run"].iter().all(|id| self.registry.get(id).is_some())
    }

    fn now(&self) -> Timestamp {
        SystemClock.now()
    }

    // -----------------------------------------------------------------------------------------------------------------
    // MCP servers

    /// One server by name.
    pub async fn server(&self, name: &str) -> Result<McpServer, HubError> {
        self.store
            .mcp_server_named(&self.tenant, name)
            .await?
            .ok_or_else(|| HubError(format!("There is no MCP server called {name}. `zorix mcp list` shows them.")))
    }

    /// Every server, by name.
    pub async fn servers(&self) -> Result<Vec<McpServer>, HubError> {
        Ok(self.store.mcp_servers(&self.tenant).await?)
    }

    /// Add a server (disabled) and keep its secrets: `(Some(variable), value)` for an environment variable of a stdio
    /// server, `(None, value)` for the bearer token of a remote one. If a secret cannot be kept, the server is not added.
    pub async fn add_server(
        &self,
        new: NewMcpServer,
        secrets: Vec<(Option<String>, Secret)>,
    ) -> Result<McpServer, HubError> {
        let mut server = self.store.add_mcp_server(&self.tenant, new, self.now()).await?;
        // A folder named to the server as one of its arguments is a folder the person gave it: its sandbox lets it in. A folder that
        // may not be given (the whole disk, a folder of credentials) is left out, and the server's sandbox shows what it has.
        if let Some(mut sandbox) = server.sandbox.clone() {
            for path in catalog().folders_named(&server.transport) {
                let one = SandboxFolder { path, write: true };
                let tried = ServerSandbox { folders: vec![one.clone()], ..sandbox.clone() };
                if tried.validate().is_ok() {
                    sandbox.folders.push(one);
                }
            }
            if !sandbox.folders.is_empty() {
                server = self.store.set_mcp_sandbox(&self.tenant, server.id, Some(sandbox), self.now()).await?;
            }
        }
        for (variable, value) in secrets {
            let name = match &variable {
                Some(variable) => env_secret_name(server.id, variable),
                None => token_secret_name(server.id),
            };
            if let Err(e) = self.keep(name, value).await {
                let _ = self.remove_server(&server.name).await;
                return Err(e);
            }
        }
        Ok(server)
    }

    /// Add a catalogue entry as a server. Returns what the person still has to provide.
    pub async fn add_from_catalog(
        &self,
        id: &str,
        name: Option<&str>,
        inputs: &std::collections::HashMap<String, String>,
    ) -> Result<(McpServer, Vec<SecretNeeded>), HubError> {
        let entry = catalog().entries.iter().find(|e| e.id == id).ok_or_else(|| {
            HubError(format!("The catalogue has no entry called {id}. `zorix mcp catalog` lists them."))
        })?;
        let (new, needed) = entry.to_new_server(name, inputs).map_err(HubError)?;
        let server = self.add_server(new, Vec::new()).await?;
        Ok((server, needed))
    }

    /// Choose how a local server is confined (ADR-0050). The server is stopped if it runs, so it starts again confined as chosen.
    pub async fn set_sandbox(&self, name: &str, sandbox: ServerSandbox) -> Result<McpServer, HubError> {
        let server = self.server(name).await?;
        if matches!(server.transport, McpTransport::Http { .. }) {
            return Err(HubError(format!(
                "{name} is a remote server: it runs elsewhere, so there is nothing here to confine."
            )));
        }
        let changed = self.store.set_mcp_sandbox(&self.tenant, server.id, Some(sandbox), self.now()).await?;
        // A running server keeps the confinement it started with, so it is stopped and, if it is on, started again.
        let running = self
            .manager
            .status(changed.id)
            .await
            .map_err(|e| HubError(e.to_string()))?
            .is_some_and(|s| !matches!(s.state, zorix_mcp::ServerState::Stopped));
        if running {
            self.manager.stop(changed.id).await;
            if changed.enabled {
                self.manager.start(changed.id).await.map_err(|e| HubError(e.to_string()))?;
            }
        }
        Ok(changed)
    }

    /// Keep one secret of a server: an environment variable (`Some`) or the bearer token (`None`).
    pub async fn set_server_secret(
        &self,
        server: &McpServer,
        variable: Option<&str>,
        value: Secret,
    ) -> Result<(), HubError> {
        match (variable, &server.transport) {
            (Some(variable), McpTransport::Stdio { env, .. }) => {
                if !env.iter().any(|e| e.name == variable && e.value == EnvValue::Secret) {
                    return Err(HubError(format!("{} does not take a secret called {variable}.", server.name)));
                }
                self.keep(env_secret_name(server.id, variable), value).await
            }
            (None, McpTransport::Http { auth, .. }) if *auth == zorix_core::HttpAuth::Bearer => {
                self.keep(token_secret_name(server.id), value).await
            }
            _ => Err(HubError(format!("{} does not take that secret.", server.name))),
        }
    }

    /// Turn a server on or off. Turning it off stops it and takes its tools away.
    pub async fn set_enabled(&self, name: &str, enabled: bool) -> Result<McpServer, HubError> {
        let mut server = self.server(name).await?;
        if server.enabled == enabled {
            return Ok(server);
        }
        server.enabled = enabled;
        let server = self.store.update_mcp_server(&self.tenant, server, self.now()).await?;
        if !enabled {
            self.manager.stop(server.id).await;
        }
        Ok(server)
    }

    /// Remove a server, its tools' records and every secret kept for it.
    pub async fn remove_server(&self, name: &str) -> Result<(), HubError> {
        let server = self.server(name).await?;
        self.manager.remove(server.id).await?;
        let base = secret_base(server.id);
        let mut names = vec![token_secret_name(server.id), format!("{base}.oauth"), format!("{base}.access")];
        if let McpTransport::Stdio { env, .. } = &server.transport {
            names.extend(
                env.iter().filter(|e| e.value == EnvValue::Secret).map(|e| env_secret_name(server.id, &e.name)),
            );
        }
        for name in names {
            let secrets = self.secrets.clone();
            let _ = tokio::task::spawn_blocking(move || secrets.remove(&name)).await;
        }
        Ok(())
    }

    /// Sign in to a remote server in the browser.
    pub async fn login(
        &self,
        name: &str,
        opener: &dyn BrowserOpener,
        cancel: &CancellationToken,
    ) -> Result<LoginSummary, HubError> {
        let server = self.server(name).await?;
        let McpTransport::Http { url, auth: zorix_core::HttpAuth::OAuth { client_id, scopes } } = &server.transport
        else {
            return Err(HubError(format!("{name} does not sign in with a browser.")));
        };
        let url = url::Url::parse(url).map_err(|_| HubError("The server's address is not valid.".into()))?;
        let client =
            OAuthClient::new(Vault::new(self.secrets.clone(), &secret_base(server.id)), Arc::new(SystemClock))?;
        let summary = client
            .login(&url, &OAuthSettings { client_id: client_id.clone(), scopes: scopes.clone() }, opener, cancel)
            .await?;
        Ok(summary)
    }

    /// Forget the sign-in of a server and stop it, so its tools go away until the next sign-in.
    pub async fn logout(&self, name: &str) -> Result<(), HubError> {
        let server = self.server(name).await?;
        Vault::new(self.secrets.clone(), &secret_base(server.id)).forget().await;
        let secrets = self.secrets.clone();
        let token = token_secret_name(server.id);
        let _ = tokio::task::spawn_blocking(move || secrets.remove(&token)).await;
        self.manager.stop(server.id).await;
        Ok(())
    }

    /// Start the enabled servers a run was granted (a grant that names `mcp.`) and wait, up to `wait`, until each has
    /// either listed its tools or given up. Returns the names of servers that are not ready, with why.
    pub async fn start_for(&self, grants: &Grants, wait: Duration) -> Result<Vec<(String, String)>, HubError> {
        let mut wanted = Vec::new();
        for server in self.servers().await?.into_iter().filter(|s| s.enabled) {
            let probe = Permission::use_tool(&format!("mcp.{}.x", server.name));
            let named = grants.iter().any(|g| g.resource.starts_with("mcp.") && g.covers(&probe));
            if named {
                self.manager.start(server.id).await?;
                wanted.push(server);
            }
        }
        let deadline = Instant::now() + wait;
        let mut trouble = Vec::new();
        loop {
            trouble.clear();
            let mut waiting = false;
            for server in &wanted {
                match self.manager.status(server.id).await?.map(|s| s.state) {
                    Some(ServerState::Ready) => {}
                    Some(ServerState::Starting) | Some(ServerState::Restarting { .. }) | None => {
                        waiting = true;
                        trouble.push((server.name.clone(), "still starting".to_string()));
                    }
                    Some(ServerState::NeedsLogin { reason }) | Some(ServerState::Failed { reason }) => {
                        trouble.push((server.name.clone(), reason));
                    }
                    Some(ServerState::Stopped) => trouble.push((server.name.clone(), "stopped".to_string())),
                }
            }
            if !waiting || Instant::now() >= deadline {
                return Ok(trouble);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Start one enabled server and wait until it has listed its tools or given up.
    pub async fn start_and_wait(&self, name: &str, wait: Duration) -> Result<zorix_mcp::ServerStatus, HubError> {
        let server = self.server(name).await?;
        self.manager.start(server.id).await?;
        let deadline = Instant::now() + wait;
        loop {
            let status = self
                .manager
                .status(server.id)
                .await?
                .ok_or_else(|| HubError(format!("{name} disappeared while starting.")))?;
            let settled = !matches!(status.state, ServerState::Starting | ServerState::Restarting { .. });
            if settled || Instant::now() >= deadline {
                return Ok(status);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    pub(crate) async fn keep(&self, name: String, value: Secret) -> Result<(), HubError> {
        let secrets = self.secrets.clone();
        tokio::task::spawn_blocking(move || secrets.set(&name, &value))
            .await
            .map_err(|e| HubError(e.to_string()))?
            .map_err(|e| HubError(format!("The secret could not be kept ({e}).")))
    }
}
