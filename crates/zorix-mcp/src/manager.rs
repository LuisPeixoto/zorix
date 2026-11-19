//! Runs the MCP servers a person added: starts them, finds out their tools, keeps the registry in step with what a person
//! approved, restarts what crashes, and says in what state each one is (ADR-0018, ADR-0020).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use zorix_core::{
    Clock, EnvValue, IntegrationStore, McpServer, McpServerId, McpToolPin, McpTransport, ObservedTool, PinAction,
    PinStatus, Result as CoreResult, SystemClock, TenantId, ToolRegistry, ToolTrust,
};
use zorix_providers::{SecretStore, get_async};

use crate::client::{CallOutput, McpClient, McpToolDef, ToolListing};
use crate::error::McpError;
use crate::stdio::{StdioConfig, StdioTransport};
use crate::tool::{McpCaller, McpTool};
use crate::transport::{ServerEvent, Transport};

/// The keychain entry that holds one environment variable of a stdio server. Names are limited to lowercase letters, digits,
/// `-`, `_` and `.`, so the variable's name is lowercased and the id shortened to its random tail.
pub fn env_secret_name(server: McpServerId, variable: &str) -> String {
    let id = server.to_string().replace('-', "");
    format!("mcp.{}.env.{}", &id[id.len() - 12..], variable.to_ascii_lowercase())
}

/// Where a server stands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ServerState {
    /// Not running.
    Stopped,
    /// Starting or finding out its tools.
    Starting,
    /// Running.
    Ready,
    /// Crashed or lost; trying again.
    Restarting {
        /// Which attempt.
        attempt: u32,
    },
    /// Needs a person to sign in.
    NeedsLogin {
        /// Why.
        reason: String,
    },
    /// Gave up.
    Failed {
        /// Why.
        reason: String,
    },
}

/// What the interface shows about a server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ServerStatus {
    /// Its id.
    pub id: McpServerId,
    /// Its name.
    pub name: String,
    /// Where it stands.
    #[serde(flatten)]
    pub state: ServerState,
    /// `modern` or `legacy`, once known.
    pub era: Option<String>,
    /// What it calls itself (self-reported).
    pub server_name: Option<String>,
    /// Its version (self-reported).
    pub server_version: Option<String>,
    /// Tools it lists.
    pub tools_listed: usize,
    /// Tools offered to runs.
    pub tools_ready: usize,
    /// Tools waiting for a person: new ones and changed ones.
    pub tools_to_review: usize,
    /// Tools left out, with why.
    pub tools_skipped: Vec<(String, String)>,
}

/// A change worth telling the interface about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusChange {
    /// The server.
    pub server: McpServerId,
}

/// How a manager behaves. The defaults are for people; tests use shorter waits.
#[derive(Clone, Debug)]
pub struct ManagerOptions {
    /// Waits between restarts; its length is the most restarts.
    pub backoff: Vec<Duration>,
    /// How long the probe waits before assuming a legacy server.
    pub probe_timeout: Duration,
    /// Where servers are started when they name no folder.
    pub work_dir: Option<PathBuf>,
}

impl Default for ManagerOptions {
    fn default() -> Self {
        Self {
            backoff: [1, 2, 4, 8, 16].into_iter().map(Duration::from_secs).collect(),
            probe_timeout: crate::client::PROBE_TIMEOUT,
            work_dir: None,
        }
    }
}

/// Makes the connection to a server. Replaceable so tests need no real servers.
#[async_trait]
pub trait Connector: Send + Sync {
    /// Connect to `server`, with its secrets already looked up by the connector itself.
    async fn connect(
        &self,
        server: &McpServer,
        secrets: &Arc<dyn SecretStore>,
        options: &ManagerOptions,
    ) -> Result<Arc<dyn Transport>, McpError>;
}

/// Starts stdio servers; remote servers arrive with the HTTP transport.
pub struct DefaultConnector;

#[async_trait]
impl Connector for DefaultConnector {
    async fn connect(
        &self,
        server: &McpServer,
        secrets: &Arc<dyn SecretStore>,
        options: &ManagerOptions,
    ) -> Result<Arc<dyn Transport>, McpError> {
        match &server.transport {
            McpTransport::Stdio { command, args, env, cwd } => {
                let mut resolved = Vec::new();
                for var in env {
                    let value = match &var.value {
                        EnvValue::Literal(text) => text.clone(),
                        EnvValue::Secret => {
                            let name = env_secret_name(server.id, &var.name);
                            get_async(secrets.clone(), name.clone())
                                .await
                                .map_err(|e| McpError::Spawn(e.to_string()))?
                                .map(|s| s.expose().to_string())
                                .ok_or_else(|| {
                                    McpError::Spawn(format!(
                                        "the value of {} is not in the keychain (it should be under “{name}”)",
                                        var.name
                                    ))
                                })?
                        }
                    };
                    resolved.push((var.name.clone(), value));
                }
                let transport = StdioTransport::spawn(StdioConfig {
                    command: command.clone(),
                    args: args.clone(),
                    env: resolved,
                    cwd: cwd.as_ref().map(PathBuf::from).or_else(|| options.work_dir.clone()),
                })?;
                Ok(Arc::new(transport))
            }
            McpTransport::Http { .. } => Err(McpError::Spawn("remote servers are not supported by this build".into())),
        }
    }
}

struct Running {
    stop: CancellationToken,
}

struct ServerRuntime {
    id: McpServerId,
    status: Mutex<ServerStatus>,
    client: RwLock<Option<Arc<McpClient>>>,
    registered: Mutex<HashSet<String>>,
    running: Mutex<Option<Running>>,
    last_log: Mutex<Vec<String>>,
}

struct Inner {
    store: Arc<dyn IntegrationStore>,
    registry: Arc<dyn ToolRegistry>,
    secrets: Arc<dyn SecretStore>,
    tenant: TenantId,
    connector: Arc<dyn Connector>,
    clock: Arc<dyn Clock>,
    options: ManagerOptions,
    servers: Mutex<HashMap<McpServerId, Arc<ServerRuntime>>>,
    changes: broadcast::Sender<StatusChange>,
}

/// Runs and supervises MCP servers.
#[derive(Clone)]
pub struct McpManager {
    inner: Arc<Inner>,
}

/// Lets a tool call the server through whatever connection is up right now.
struct Handle {
    runtime: Arc<ServerRuntime>,
}

#[async_trait]
impl McpCaller for Handle {
    async fn call(
        &self,
        tool: &str,
        arguments: Value,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<CallOutput, McpError> {
        let client = self.runtime.client.read().clone();
        match client {
            Some(client) if client.alive() => client.call_tool(tool, arguments, timeout, cancel).await,
            _ => Err(McpError::Closed),
        }
    }
}

fn counts(pins: &[McpToolPin]) -> (usize, usize, usize) {
    let listed = pins.iter().filter(|p| p.listed).count();
    let ready = pins.iter().filter(|p| p.usable()).count();
    let review = pins.iter().filter(|p| p.listed && matches!(p.status, PinStatus::New | PinStatus::Changed)).count();
    (listed, ready, review)
}

/// Rebuild a definition from what was stored when the server listed it.
fn definition_from(stored: &Value) -> Option<McpToolDef> {
    let name = stored.get("name")?.as_str()?.to_string();
    Some(McpToolDef {
        name,
        title: stored.get("title").and_then(Value::as_str).map(str::to_string),
        description: stored.get("description").and_then(Value::as_str).unwrap_or("").to_string(),
        input_schema: stored.get("inputSchema").cloned().unwrap_or_else(|| serde_json::json!({"type": "object"})),
        output_schema: stored.get("outputSchema").filter(|s| !s.is_null()).cloned(),
        annotations: stored.get("annotations").cloned().unwrap_or_else(|| serde_json::json!({})),
    })
}

impl McpManager {
    /// A manager for one tenant's servers, registering their tools into `registry`.
    pub fn new(
        store: Arc<dyn IntegrationStore>,
        registry: Arc<dyn ToolRegistry>,
        secrets: Arc<dyn SecretStore>,
        tenant: TenantId,
    ) -> Self {
        Self::with(
            store,
            registry,
            secrets,
            tenant,
            Arc::new(DefaultConnector),
            Arc::new(SystemClock),
            ManagerOptions::default(),
        )
    }

    /// The same with every part chosen.
    pub fn with(
        store: Arc<dyn IntegrationStore>,
        registry: Arc<dyn ToolRegistry>,
        secrets: Arc<dyn SecretStore>,
        tenant: TenantId,
        connector: Arc<dyn Connector>,
        clock: Arc<dyn Clock>,
        options: ManagerOptions,
    ) -> Self {
        let (changes, _) = broadcast::channel(64);
        Self {
            inner: Arc::new(Inner {
                store,
                registry,
                secrets,
                tenant,
                connector,
                clock,
                options,
                servers: Mutex::new(HashMap::new()),
                changes,
            }),
        }
    }

    /// Follow changes in the state of any server.
    pub fn subscribe(&self) -> broadcast::Receiver<StatusChange> {
        self.inner.changes.subscribe()
    }

    fn runtime(&self, server: &McpServer) -> Arc<ServerRuntime> {
        self.inner
            .servers
            .lock()
            .entry(server.id)
            .or_insert_with(|| {
                Arc::new(ServerRuntime {
                    id: server.id,
                    status: Mutex::new(ServerStatus {
                        id: server.id,
                        name: server.name.clone(),
                        state: ServerState::Stopped,
                        era: server.era.clone(),
                        server_name: None,
                        server_version: None,
                        tools_listed: 0,
                        tools_ready: 0,
                        tools_to_review: 0,
                        tools_skipped: Vec::new(),
                    }),
                    client: RwLock::new(None),
                    registered: Mutex::new(HashSet::new()),
                    running: Mutex::new(None),
                    last_log: Mutex::new(Vec::new()),
                })
            })
            .clone()
    }

    fn set_state(&self, runtime: &ServerRuntime, state: ServerState) {
        runtime.status.lock().state = state;
        let _ = self.inner.changes.send(StatusChange { server: runtime.id });
    }

    /// The status of every server the tenant has, running or not.
    pub async fn statuses(&self) -> CoreResult<Vec<ServerStatus>> {
        let mut all = Vec::new();
        for server in self.inner.store.mcp_servers(&self.inner.tenant).await? {
            all.push(self.runtime(&server).status.lock().clone());
        }
        Ok(all)
    }

    /// The status of one server.
    pub async fn status(&self, id: McpServerId) -> CoreResult<Option<ServerStatus>> {
        Ok(self.inner.store.mcp_server(&self.inner.tenant, id).await?.map(|s| self.runtime(&s).status.lock().clone()))
    }

    /// What the server said on its error stream (the last run's, when it is not running).
    pub fn log(&self, id: McpServerId) -> Vec<String> {
        let Some(runtime) = self.inner.servers.lock().get(&id).cloned() else { return Vec::new() };
        let live = runtime.client.read().as_ref().map(|c| c.log());
        live.unwrap_or_else(|| runtime.last_log.lock().clone())
    }

    /// Start every enabled server.
    pub async fn start_all(&self) -> CoreResult<()> {
        for server in self.inner.store.mcp_servers(&self.inner.tenant).await? {
            if server.enabled {
                self.start(server.id).await?;
            }
        }
        Ok(())
    }

    /// Start a server (it must be enabled). Does nothing if it is already running.
    pub async fn start(&self, id: McpServerId) -> CoreResult<()> {
        let server = self
            .inner
            .store
            .mcp_server(&self.inner.tenant, id)
            .await?
            .ok_or_else(|| zorix_core::Error::Missing(format!("MCP server {id}")))?;
        if !server.enabled {
            return Err(zorix_core::Error::InvalidInput(format!("{} is not enabled.", server.name)));
        }
        let runtime = self.runtime(&server);
        let stop = {
            let mut running = runtime.running.lock();
            if running.is_some() {
                return Ok(());
            }
            let stop = CancellationToken::new();
            *running = Some(Running { stop: stop.clone() });
            stop
        };
        self.set_state(&runtime, ServerState::Starting);
        let manager = self.clone();
        tokio::spawn(async move { manager.supervise(server, runtime, stop).await });
        Ok(())
    }

    /// Stop a server and take its tools away.
    pub async fn stop(&self, id: McpServerId) {
        let Some(runtime) = self.inner.servers.lock().get(&id).cloned() else { return };
        let running = runtime.running.lock().take();
        if let Some(running) = running {
            running.stop.cancel();
        }
        self.drop_connection(&runtime).await;
        self.unregister_all(&runtime);
        self.set_state(&runtime, ServerState::Stopped);
    }

    /// Stop everything, for shutdown.
    pub async fn shutdown(&self) {
        let ids: Vec<McpServerId> = self.inner.servers.lock().keys().copied().collect();
        for id in ids {
            self.stop(id).await;
        }
    }

    /// Forget a server entirely.
    pub async fn remove(&self, id: McpServerId) -> CoreResult<bool> {
        self.stop(id).await;
        self.inner.servers.lock().remove(&id);
        self.inner.store.remove_mcp_server(&self.inner.tenant, id).await
    }

    /// Ask a running server for its tools again.
    pub async fn refresh(&self, id: McpServerId) -> Result<(), McpError> {
        let runtime = self.inner.servers.lock().get(&id).cloned().ok_or(McpError::Closed)?;
        let client = runtime.client.read().clone().ok_or(McpError::Closed)?;
        self.sync_tools(&runtime, &client).await
    }

    /// A person approves what a server lists for one tool, or turns it off.
    pub async fn review(&self, id: McpServerId, tool: &str, action: PinAction, by: &str) -> CoreResult<McpToolPin> {
        let pin =
            self.inner.store.review_tool(&self.inner.tenant, id, tool, action, by, self.inner.clock.now()).await?;
        self.register_from_pins(id).await?;
        Ok(pin)
    }

    /// A person approves every tool that is new or changed.
    pub async fn approve_all(&self, id: McpServerId, by: &str) -> CoreResult<usize> {
        let pins = self.inner.store.tool_pins(&self.inner.tenant, id).await?;
        let mut approved = 0;
        for pin in pins.iter().filter(|p| p.listed && matches!(p.status, PinStatus::New | PinStatus::Changed)) {
            self.inner
                .store
                .review_tool(&self.inner.tenant, id, &pin.tool, PinAction::Approve, by, self.inner.clock.now())
                .await?;
            approved += 1;
        }
        self.register_from_pins(id).await?;
        Ok(approved)
    }

    /// Whether a tool asks before it runs.
    pub async fn set_trust(&self, id: McpServerId, tool: &str, trust: ToolTrust) -> CoreResult<McpToolPin> {
        let pin = self.inner.store.set_tool_trust(&self.inner.tenant, id, tool, trust).await?;
        self.register_from_pins(id).await?;
        Ok(pin)
    }

    async fn drop_connection(&self, runtime: &ServerRuntime) {
        let client = runtime.client.write().take();
        if let Some(client) = client {
            *runtime.last_log.lock() = client.log();
            client.close().await;
        }
    }

    fn unregister_all(&self, runtime: &ServerRuntime) {
        let ids: Vec<String> = runtime.registered.lock().drain().collect();
        for id in ids {
            self.inner.registry.unregister(&id);
        }
    }

    /// Make the registry hold exactly the tools a person approved, from what was stored.
    async fn register_from_pins(&self, id: McpServerId) -> CoreResult<()> {
        let Some(server) = self.inner.store.mcp_server(&self.inner.tenant, id).await? else { return Ok(()) };
        let runtime = self.runtime(&server);
        let pins = self.inner.store.tool_pins(&self.inner.tenant, id).await?;
        // Only a running server has tools to offer.
        let running = runtime.client.read().as_ref().is_some_and(|c| c.alive());
        let mut wanted = HashSet::new();
        if running {
            let caller: Arc<dyn McpCaller> = Arc::new(Handle { runtime: runtime.clone() });
            for pin in pins.iter().filter(|p| p.usable()) {
                let Some(definition) = definition_from(pin.approved.as_ref().unwrap_or(&pin.seen)) else { continue };
                let tool = McpTool::new(&server.name, &definition, pin.trust, caller.clone());
                let tool_id = zorix_core::Tool::spec(&tool).id.clone();
                if self.inner.registry.replace(Arc::new(tool)).is_ok() {
                    wanted.insert(tool_id);
                }
            }
        }
        let stale: Vec<String> = runtime.registered.lock().iter().filter(|t| !wanted.contains(*t)).cloned().collect();
        for tool_id in stale {
            self.inner.registry.unregister(&tool_id);
        }
        *runtime.registered.lock() = wanted;
        let (listed, ready, review) = counts(&pins);
        {
            let mut status = runtime.status.lock();
            status.tools_listed = listed;
            status.tools_ready = ready;
            status.tools_to_review = review;
        }
        let _ = self.inner.changes.send(StatusChange { server: id });
        Ok(())
    }

    async fn sync_tools(&self, runtime: &Arc<ServerRuntime>, client: &Arc<McpClient>) -> Result<(), McpError> {
        let listing: ToolListing = client.list_tools(&CancellationToken::new()).await?;
        let observed: Vec<ObservedTool> = listing
            .tools
            .iter()
            .map(|t| ObservedTool { name: t.name.clone(), hash: t.hash(), definition: t.definition() })
            .collect();
        self.inner
            .store
            .observe_tools(&self.inner.tenant, runtime.id, observed, self.inner.clock.now())
            .await
            .map_err(|e| McpError::Protocol(e.to_string()))?;
        runtime.status.lock().tools_skipped = listing.skipped;
        self.register_from_pins(runtime.id).await.map_err(|e| McpError::Protocol(e.to_string()))
    }

    /// One life of a server: connect, find the tools, watch it, and start again when it dies.
    async fn supervise(&self, server: McpServer, runtime: Arc<ServerRuntime>, stop: CancellationToken) {
        let mut attempt = 0usize;
        loop {
            let outcome = self.run_once(&server, &runtime, &stop).await;
            self.drop_connection(&runtime).await;
            if stop.is_cancelled() {
                return;
            }
            let reason = match outcome {
                Ok(()) => "the server stopped".to_string(),
                Err(McpError::NeedsLogin(reason)) => {
                    self.unregister_all(&runtime);
                    self.set_state(&runtime, ServerState::NeedsLogin { reason });
                    *runtime.running.lock() = None;
                    return;
                }
                Err(error) => error.to_string(),
            };
            self.unregister_all(&runtime);
            let Some(wait) = self.inner.options.backoff.get(attempt).copied() else {
                self.set_state(&runtime, ServerState::Failed { reason });
                *runtime.running.lock() = None;
                return;
            };
            attempt += 1;
            self.set_state(&runtime, ServerState::Restarting { attempt: attempt as u32 });
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = stop.cancelled() => return,
            }
        }
    }

    async fn run_once(
        &self,
        server: &McpServer,
        runtime: &Arc<ServerRuntime>,
        stop: &CancellationToken,
    ) -> Result<(), McpError> {
        let transport = self.inner.connector.connect(server, &self.inner.secrets, &self.inner.options).await?;
        let client = McpClient::connect_with(
            transport,
            stop,
            self.inner.options.probe_timeout,
            crate::client::HANDSHAKE_TIMEOUT,
        )
        .await;
        let client = match client {
            Ok(client) => Arc::new(client),
            Err(error) => return Err(error),
        };
        let mut events = client.events();
        *runtime.client.write() = Some(client.clone());
        {
            let mut status = runtime.status.lock();
            status.era = Some(client.era().label().to_string());
            status.server_name = client.info().name.clone();
            status.server_version = client.info().version.clone();
        }
        let _ =
            self.inner.store.set_mcp_era(&self.inner.tenant, server.id, Some(client.era().label().to_string())).await;
        self.sync_tools(runtime, &client).await?;
        self.set_state(runtime, ServerState::Ready);
        loop {
            tokio::select! {
                _ = stop.cancelled() => return Ok(()),
                event = events.recv() => match event {
                    Ok(ServerEvent::Notification { method, .. }) if method == "notifications/tools/list_changed" => {
                        self.sync_tools(runtime, &client).await?;
                    }
                    Ok(ServerEvent::Closed(why)) => return Err(McpError::Transport(why)),
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return Err(McpError::Closed),
                },
            }
        }
    }
}
