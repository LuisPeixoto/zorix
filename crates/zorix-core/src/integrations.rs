//! What ZORIX knows about the outside world: MCP servers a person added, the definitions of their tools a person
//! approved, and connections to services. Secrets are never here: a server or connection names a keychain entry and the
//! secret itself stays in the keychain (ADR-0011, ADR-0018, ADR-0019).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::Result;
use crate::ids::{ConnectionId, McpServerId, TenantId};
use crate::time::Timestamp;

/// Most MCP servers per tenant.
pub const MAX_MCP_SERVERS: usize = 50;
/// Most tools of one server that are kept.
pub const MAX_TOOLS_PER_SERVER: usize = 200;
/// Most connections per tenant.
pub const MAX_CONNECTIONS: usize = 100;

/// Names of servers and connections: they become part of tool ids and grants, so they stay plain.
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else { return false };
    name.len() <= 32
        && (first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// An environment variable handed to a stdio server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvVar {
    /// Its name.
    pub name: String,
    /// Its value, or where to find it.
    pub value: EnvValue,
}

/// Check the environment variables of a stdio server: each name is an ordinary variable name, and no two differ only by case. Windows
/// reads `FOO` and `foo` as one variable, and ZORIX keeps one secret per name written in lowercase, so two such names would share a
/// secret and one value would silently replace the other.
pub fn validate_env(vars: &[EnvVar]) -> Result<(), String> {
    let mut seen: std::collections::HashMap<String, &str> = std::collections::HashMap::new();
    for var in vars {
        let ok = !var.name.is_empty()
            && var.name.len() <= 128
            && var.name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && var.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !ok {
            return Err(format!(
                "“{}” is not a variable name: use letters, digits and '_', not starting with a digit.",
                var.name
            ));
        }
        if let Some(first) = seen.insert(var.name.to_ascii_lowercase(), &var.name) {
            return Err(if first == var.name {
                format!("The variable {first} is listed twice.")
            } else {
                format!(
                    "The variables {first} and {} differ only by case, which Windows reads as one variable and ZORIX keeps one secret for. Keep one of them.",
                    var.name
                )
            });
        }
    }
    Ok(())
}

/// The value of an [`EnvVar`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum EnvValue {
    /// Not a secret: stored as written.
    Literal(String),
    /// A secret: the value is in the keychain.
    Secret,
}

/// How ZORIX signs in to a remote server.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HttpAuth {
    /// No sign-in.
    #[default]
    None,
    /// A bearer token kept in the keychain.
    Bearer,
    /// OAuth 2.1 (ADR-0019). Tokens are kept in the keychain.
    OAuth {
        /// A client id the person supplied, when the server needs one registered in advance.
        client_id: Option<String>,
        /// Scopes to ask for, when the person chose.
        scopes: Vec<String>,
    },
}

/// How to reach an MCP server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpTransport {
    /// A program ZORIX starts and talks to over its standard streams, sandboxed by default (see [`ServerSandbox`]).
    Stdio {
        /// The program.
        command: String,
        /// Its arguments.
        args: Vec<String>,
        /// Environment variables it gets besides a minimal clean environment.
        env: Vec<EnvVar>,
        /// The working directory, if not the data folder.
        cwd: Option<String>,
    },
    /// A server reached over Streamable HTTP.
    Http {
        /// The MCP endpoint.
        url: String,
        /// How to sign in.
        auth: HttpAuth,
    },
}

/// How a local server is confined.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxMode {
    /// Restricted with what this operating system can enforce.
    Native,
    /// Not restricted: the server runs as the person, as every server did before Phase 12.
    Off,
}

/// A folder a confined server may use besides its own.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxFolder {
    /// The absolute path.
    pub path: String,
    /// Whether the server may change what is in it.
    #[serde(default)]
    pub write: bool,
}

/// The sandbox of a local (stdio) MCP server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerSandbox {
    /// Whether and how it is confined.
    pub mode: SandboxMode,
    /// Whether it may open network connections.
    pub network: bool,
    /// Folders it may use besides its own.
    #[serde(default)]
    pub folders: Vec<SandboxFolder>,
}

/// The most folders a confined server may be given.
pub const MAX_SANDBOX_FOLDERS: usize = 16;

impl ServerSandbox {
    /// The sandbox a new local server starts with: confined, and with the network only when its command has to fetch what it runs (`npx`,
    /// `uvx` and the like download the package on first use).
    pub fn for_new(command: &str) -> Self {
        let name = command.rsplit(['/', '\\']).next().unwrap_or(command).to_ascii_lowercase();
        let name = name.strip_suffix(".cmd").or_else(|| name.strip_suffix(".exe")).unwrap_or(&name).to_string();
        let fetches = matches!(name.as_str(), "npx" | "uvx" | "bunx" | "pnpm" | "yarn" | "uv" | "pipx" | "deno");
        Self { mode: SandboxMode::Native, network: fetches, folders: Vec::new() }
    }

    /// Check the folders: each an absolute path with no `..`, not a filesystem root, not a folder that holds credentials.
    pub fn validate(&self) -> Result<(), String> {
        if self.folders.len() > MAX_SANDBOX_FOLDERS {
            return Err(format!("A server may be given at most {MAX_SANDBOX_FOLDERS} folders."));
        }
        for folder in &self.folders {
            let path = std::path::Path::new(&folder.path);
            let text = &folder.path;
            if !path.is_absolute() || text.chars().any(char::is_control) || text.contains('"') {
                return Err(format!("“{text}” is not a folder ZORIX can give a server: use an absolute path."));
            }
            if path.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
                return Err(format!("“{text}” goes up with “..”; write the folder itself."));
            }
            if path.parent().is_none() {
                return Err(format!("“{text}” is the whole disk. Give a server only the folder it needs."));
            }
            let private = [".ssh", ".aws", ".gnupg", ".kube", ".docker", ".azure", "gcloud", "Keychains"];
            if path.components().any(|c| private.iter().any(|p| c.as_os_str() == *p)) {
                return Err(format!("“{text}” holds credentials and is never given to a server."));
            }
        }
        Ok(())
    }
}

/// An MCP server a person added.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServer {
    /// Its id.
    pub id: McpServerId,
    /// The owner.
    pub tenant: TenantId,
    /// ZORIX's name for it, used in tool ids (`mcp.<name>.<tool>`) and grants.
    pub name: String,
    /// How to reach it.
    pub transport: McpTransport,
    /// Whether it is started and its tools offered.
    pub enabled: bool,
    /// The protocol era found the last time (`modern` or `legacy`), cached (ADR-0018).
    pub era: Option<String>,
    /// The sandbox of a local server. `None` for a remote one, and for a local one added before ZORIX had sandboxes, which runs
    /// unconfined until a person chooses.
    #[serde(default)]
    pub sandbox: Option<ServerSandbox>,
    /// When it was added.
    pub created_at: Timestamp,
    /// When it was last changed.
    pub updated_at: Timestamp,
}

/// An MCP server to add. It starts disabled: a person enables it after seeing what it is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewMcpServer {
    /// Its name.
    pub name: String,
    /// How to reach it.
    pub transport: McpTransport,
}

/// Where a tool's definition stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinStatus {
    /// The server lists it and no person has approved any definition yet. Not offered.
    New,
    /// A person approved exactly the definition the server lists now. Offered.
    Pinned,
    /// The server lists a different definition from the one approved. Not offered until a person reviews the change.
    Changed,
    /// A person turned it off. Not offered.
    Disabled,
}

impl PinStatus {
    /// The name stored.
    pub fn as_str(&self) -> &'static str {
        match self {
            PinStatus::New => "new",
            PinStatus::Pinned => "pinned",
            PinStatus::Changed => "changed",
            PinStatus::Disabled => "disabled",
        }
    }

    /// Read a stored name.
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "new" => PinStatus::New,
            "pinned" => PinStatus::Pinned,
            "changed" => PinStatus::Changed,
            "disabled" => PinStatus::Disabled,
            _ => return None,
        })
    }
}

/// Whether a tool asks before it runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolTrust {
    /// Every call asks (the default).
    #[default]
    Ask,
    /// Calls do not ask. Only allowed for a tool the server reports as read-only (ADR-0018).
    ReadOnlyNoAsk,
}

impl ToolTrust {
    /// The name stored.
    pub fn as_str(&self) -> &'static str {
        match self {
            ToolTrust::Ask => "ask",
            ToolTrust::ReadOnlyNoAsk => "read_only_no_ask",
        }
    }

    /// Read a stored name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "ask" => Some(ToolTrust::Ask),
            "read_only_no_ask" => Some(ToolTrust::ReadOnlyNoAsk),
            _ => None,
        }
    }
}

/// What a person decides about a tool's definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PinAction {
    /// Approve a definition. With a hash, it is approved only if that is still the definition the server lists: what the person looked
    /// at is what is approved, and a server that changes the tool between the look and the click gets a refusal, not an approval. With
    /// no hash it is whatever the server lists now, which is for a person who is deliberately approving "everything pending".
    Approve {
        /// The hash of the definition that was shown.
        hash: Option<String>,
    },
    /// Turn the tool off.
    Disable,
}

impl PinAction {
    /// Approve whatever the server lists now.
    pub fn approve() -> Self {
        Self::Approve { hash: None }
    }

    /// Approve the definition with this hash, and nothing else.
    pub fn approve_seen(hash: impl Into<String>) -> Self {
        Self::Approve { hash: Some(hash.into()) }
    }
}

/// A tool's definition as a server listed it, for [`IntegrationStore::observe_tools`].
#[derive(Clone, Debug, PartialEq)]
pub struct ObservedTool {
    /// The name the server uses.
    pub name: String,
    /// Its [`ToolSpec::definition_hash`](crate::ToolSpec::definition_hash)-style hash.
    pub hash: String,
    /// The definition (name, description, schemas, annotations) as listed.
    pub definition: Value,
}

/// The record of one tool of one server: what was approved and what the server lists now.
#[derive(Clone, Debug, PartialEq)]
pub struct McpToolPin {
    /// The server.
    pub server: McpServerId,
    /// The tool's name at the server.
    pub tool: String,
    /// The server lists it right now.
    pub listed: bool,
    /// Where it stands.
    pub status: PinStatus,
    /// Whether it asks before running.
    pub trust: ToolTrust,
    /// The hash a person approved.
    pub approved_hash: Option<String>,
    /// The definition a person approved.
    pub approved: Option<Value>,
    /// The hash the server lists now (or listed last).
    pub seen_hash: String,
    /// The definition the server lists now (or listed last).
    pub seen: Value,
    /// Who approved it.
    pub approved_by: Option<String>,
    /// When.
    pub approved_at: Option<Timestamp>,
    /// When this record last changed.
    pub updated_at: Timestamp,
}

impl McpToolPin {
    /// True when the tool may be offered to runs.
    pub fn usable(&self) -> bool {
        self.listed && self.status == PinStatus::Pinned
    }
}

/// The kinds of connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionKind {
    /// A web API reached with `http.get` and `http.request`.
    Http,
    /// A URL that `webhook.send` posts signed events to.
    Webhook,
    /// GitHub.
    Github,
    /// GitLab.
    Gitlab,
    /// Jira.
    Jira,
    /// Slack.
    Slack,
    /// Discord.
    Discord,
    /// A Postgres database.
    Postgres,
    /// A SQLite file.
    Sqlite,
}

impl ConnectionKind {
    /// The name stored and used on the command line.
    pub fn as_str(&self) -> &'static str {
        match self {
            ConnectionKind::Http => "http",
            ConnectionKind::Webhook => "webhook",
            ConnectionKind::Github => "github",
            ConnectionKind::Gitlab => "gitlab",
            ConnectionKind::Jira => "jira",
            ConnectionKind::Slack => "slack",
            ConnectionKind::Discord => "discord",
            ConnectionKind::Postgres => "postgres",
            ConnectionKind::Sqlite => "sqlite",
        }
    }

    /// Read a name.
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "http" => ConnectionKind::Http,
            "webhook" => ConnectionKind::Webhook,
            "github" => ConnectionKind::Github,
            "gitlab" => ConnectionKind::Gitlab,
            "jira" => ConnectionKind::Jira,
            "slack" => ConnectionKind::Slack,
            "discord" => ConnectionKind::Discord,
            "postgres" => ConnectionKind::Postgres,
            "sqlite" => ConnectionKind::Sqlite,
            _ => return None,
        })
    }
}

/// The result of the last test of a connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionTest {
    /// When it ran.
    pub at: Timestamp,
    /// Whether it worked.
    pub ok: bool,
    /// What it found, or why it failed (no secrets).
    pub note: String,
}

/// A connection to an outside service.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Connection {
    /// Its id.
    pub id: ConnectionId,
    /// The owner.
    pub tenant: TenantId,
    /// What it connects to.
    pub kind: ConnectionKind,
    /// Its name, used in grants (`db:<name>`).
    pub name: String,
    /// Settings that are not secret: base URL, owner, host, database, user…
    pub settings: Value,
    /// A secret for it is kept in the keychain.
    pub has_secret: bool,
    /// When it was added.
    pub created_at: Timestamp,
    /// When it was last changed.
    pub updated_at: Timestamp,
    /// The last test.
    pub last_test: Option<ConnectionTest>,
}

/// A connection to add.
#[derive(Clone, Debug, PartialEq)]
pub struct NewConnection {
    /// What it connects to.
    pub kind: ConnectionKind,
    /// Its name.
    pub name: String,
    /// Settings that are not secret.
    pub settings: Value,
    /// A secret is kept in the keychain.
    pub has_secret: bool,
}

/// Changes to a connection. `None` leaves a field alone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConnectionPatch {
    /// New settings.
    pub settings: Option<Value>,
    /// Whether a secret is kept.
    pub has_secret: Option<bool>,
}

/// Where integrations are remembered. Every call carries the tenant; nothing crosses tenants.
#[async_trait]
pub trait IntegrationStore: Send + Sync {
    /// The MCP servers of a tenant, by name.
    async fn mcp_servers(&self, tenant: &TenantId) -> Result<Vec<McpServer>>;
    /// One server.
    async fn mcp_server(&self, tenant: &TenantId, id: McpServerId) -> Result<Option<McpServer>>;
    /// One server by name.
    async fn mcp_server_named(&self, tenant: &TenantId, name: &str) -> Result<Option<McpServer>>;
    /// Add a server (disabled). Fails on a bad or taken name, or when the tenant has [`MAX_MCP_SERVERS`].
    async fn add_mcp_server(&self, tenant: &TenantId, new: NewMcpServer, now: Timestamp) -> Result<McpServer>;
    /// Change a server's transport or whether it is enabled. The name does not change.
    async fn update_mcp_server(&self, tenant: &TenantId, server: McpServer, now: Timestamp) -> Result<McpServer>;
    /// Set how a local server is confined. `None` means "never chosen".
    async fn set_mcp_sandbox(
        &self,
        tenant: &TenantId,
        id: McpServerId,
        sandbox: Option<ServerSandbox>,
        now: Timestamp,
    ) -> Result<McpServer>;
    /// Remember the protocol era found for a server.
    async fn set_mcp_era(&self, tenant: &TenantId, id: McpServerId, era: Option<String>) -> Result<()>;
    /// Remove a server and what is known about its tools.
    async fn remove_mcp_server(&self, tenant: &TenantId, id: McpServerId) -> Result<bool>;

    /// What is known about the tools of a server, by name.
    async fn tool_pins(&self, tenant: &TenantId, server: McpServerId) -> Result<Vec<McpToolPin>>;
    /// Record what the server lists now. New tools arrive as `New`; an approved tool whose hash differs becomes
    /// `Changed`; one whose hash is back to the approved one becomes `Pinned` again; tools no longer listed are marked so.
    /// Fails when more than [`MAX_TOOLS_PER_SERVER`] tools are listed.
    async fn observe_tools(
        &self,
        tenant: &TenantId,
        server: McpServerId,
        observed: Vec<ObservedTool>,
        now: Timestamp,
    ) -> Result<Vec<McpToolPin>>;
    /// A person approves the definition listed now, or turns the tool off.
    async fn review_tool(
        &self,
        tenant: &TenantId,
        server: McpServerId,
        tool: &str,
        action: PinAction,
        by: &str,
        now: Timestamp,
    ) -> Result<McpToolPin>;
    /// Whether a tool asks. `ReadOnlyNoAsk` is refused unless the listed definition's annotations say `readOnlyHint` and
    /// not `destructiveHint` or `openWorldHint`.
    async fn set_tool_trust(
        &self,
        tenant: &TenantId,
        server: McpServerId,
        tool: &str,
        trust: ToolTrust,
    ) -> Result<McpToolPin>;

    /// The connections of a tenant, by name.
    async fn connections(&self, tenant: &TenantId) -> Result<Vec<Connection>>;
    /// One connection by id.
    async fn connection(&self, tenant: &TenantId, id: ConnectionId) -> Result<Option<Connection>>;
    /// One connection by name.
    async fn connection_named(&self, tenant: &TenantId, name: &str) -> Result<Option<Connection>>;
    /// Add a connection. Fails on a bad or taken name, or at [`MAX_CONNECTIONS`].
    async fn add_connection(&self, tenant: &TenantId, new: NewConnection, now: Timestamp) -> Result<Connection>;
    /// Change a connection.
    async fn update_connection(
        &self,
        tenant: &TenantId,
        id: ConnectionId,
        patch: ConnectionPatch,
        now: Timestamp,
    ) -> Result<Connection>;
    /// Keep the result of testing a connection.
    async fn record_connection_test(&self, tenant: &TenantId, id: ConnectionId, test: ConnectionTest) -> Result<()>;
    /// Remove a connection.
    async fn remove_connection(&self, tenant: &TenantId, id: ConnectionId) -> Result<bool>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_plain_because_they_become_tool_ids_and_grants() {
        for good in ["github", "my-db", "a1", "x_y", "0day"] {
            assert!(valid_name(good), "{good}");
        }
        for bad in ["", "GitHub", "-x", "a b", "a.b", "a:b", "é", &"x".repeat(33)] {
            assert!(!valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn stored_names_round_trip() {
        for status in [PinStatus::New, PinStatus::Pinned, PinStatus::Changed, PinStatus::Disabled] {
            assert_eq!(PinStatus::parse(status.as_str()), Some(status));
        }
        for kind in ["http", "webhook", "github", "gitlab", "jira", "slack", "discord", "postgres", "sqlite"] {
            assert_eq!(ConnectionKind::parse(kind).unwrap().as_str(), kind);
        }
        assert_eq!(ToolTrust::parse("read_only_no_ask"), Some(ToolTrust::ReadOnlyNoAsk));
    }

    #[test]
    fn a_stored_transport_never_holds_a_secret_value() {
        let transport = McpTransport::Stdio {
            command: "npx".into(),
            args: vec!["server".into()],
            env: vec![EnvVar { name: "TOKEN".into(), value: EnvValue::Secret }],
            cwd: None,
        };
        let text = serde_json::to_string(&transport).unwrap();
        assert!(text.contains("TOKEN") && text.contains("secret") && !text.contains("value\":\"ghp"));
        assert_eq!(serde_json::from_str::<McpTransport>(&text).unwrap(), transport);
    }

    #[test]
    fn a_new_server_is_confined_and_has_the_network_only_to_fetch_what_it_runs() {
        for fetching in ["npx", "uvx", "/usr/local/bin/npx", "C:\\Program Files\\nodejs\\npx.cmd", "UVX"] {
            let sandbox = ServerSandbox::for_new(fetching);
            assert_eq!((sandbox.mode, sandbox.network), (SandboxMode::Native, true), "{fetching}");
        }
        for local in ["node", "python3", "./server", "zorix-fake-mcp", "npxish"] {
            let sandbox = ServerSandbox::for_new(local);
            assert_eq!((sandbox.mode, sandbox.network), (SandboxMode::Native, false), "{local}");
        }
    }

    #[test]
    fn a_folder_for_a_server_is_absolute_narrow_and_never_a_credential_folder() {
        let with = |path: &str| ServerSandbox {
            mode: SandboxMode::Native,
            network: false,
            folders: vec![SandboxFolder { path: path.into(), write: false }],
        };
        let root = if cfg!(windows) { "C:\\" } else { "/" };
        let fine = if cfg!(windows) { "C:\\work\\notes" } else { "/work/notes" };
        assert!(with(fine).validate().is_ok());
        for bad in [
            root.to_string(),
            "notes".into(),
            "../x".into(),
            format!("{fine}/../.."),
            format!("{root}home/a/.ssh"),
            format!("{root}home/a/.config/gcloud"),
            format!("{fine}\""),
        ] {
            assert!(with(&bad).validate().is_err(), "{bad}");
        }
        let many = ServerSandbox {
            folders: vec![SandboxFolder { path: fine.into(), write: false }; MAX_SANDBOX_FOLDERS + 1],
            ..with(fine)
        };
        assert!(many.validate().is_err());
    }
}
