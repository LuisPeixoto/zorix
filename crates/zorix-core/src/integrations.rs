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
    /// A program ZORIX starts and talks to over its standard streams. It runs as the user, without a sandbox.
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinAction {
    /// Approve the definition the server lists now.
    Approve,
    /// Turn the tool off.
    Disable,
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
}
