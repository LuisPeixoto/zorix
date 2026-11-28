//! The SQLite implementation of [`IntegrationStore`] (ADR-0018, ADR-0021). It opens its own connection, like the memory
//! store, and every query carries the tenant. No secret is stored here.

use std::path::Path;
use std::sync::Arc;

use crate::errors::storage;
use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::{Connection as Db, OptionalExtension, Row, params};
use serde_json::Value;
use zorix_core::integrations::{MAX_CONNECTIONS, MAX_MCP_SERVERS, MAX_TOOLS_PER_SERVER};
use zorix_core::{
    Connection, ConnectionId, ConnectionKind, ConnectionPatch, ConnectionTest, Error, IntegrationStore, McpServer,
    McpServerId, McpToolPin, McpTransport, NewConnection, NewMcpServer, ObservedTool, PinAction, PinStatus, Result,
    ServerSandbox, TenantId, Timestamp, ToolTrust, valid_name,
};

/// Stores integrations in the same SQLite file as everything else.
#[derive(Clone)]
pub struct SqliteIntegrationStore {
    conn: Arc<Mutex<Db>>,
}

fn invalid(what: impl Into<String>) -> Error {
    Error::InvalidInput(what.into())
}

fn parse<T: serde::de::DeserializeOwned>(text: &str) -> Result<T> {
    serde_json::from_str(text).map_err(|e| Error::Storage(format!("a stored record could not be read: {e}")))
}

fn encode<T: serde::Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(storage)
}

impl SqliteIntegrationStore {
    /// Open the database at `path` (creating and migrating it when needed) with its own connection.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Storage(format!("could not create {}: {e}", parent.display())))?;
        }
        Self::prepare(Db::open(path).map_err(storage)?, true)
    }

    /// A database in memory, for tests.
    pub fn open_in_memory() -> Result<Self> {
        Self::prepare(Db::open_in_memory().map_err(storage)?, false)
    }

    fn prepare(mut conn: Db, file: bool) -> Result<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(storage)?;
        if file {
            conn.pragma_update(None, "journal_mode", "WAL").map_err(storage)?;
            conn.pragma_update(None, "synchronous", "NORMAL").map_err(storage)?;
        }
        conn.pragma_update(None, "secure_delete", "ON").map_err(storage)?;
        crate::schema::migrate(&mut conn).map_err(Error::Storage)?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    fn with<T>(&self, work: impl FnOnce(&mut Db) -> Result<T>) -> Result<T> {
        let mut conn = self.conn.lock();
        work(&mut conn)
    }
}

const SERVER_COLUMNS: &str = "id, tenant, name, enabled, era, created_at, updated_at, doc, sandbox";

/// The columns of a server row as read: id, tenant, name, enabled, era, created, updated, transport JSON, sandbox JSON.
type RawServer = (String, String, String, bool, Option<String>, i64, i64, String, Option<String>);

fn server_from(row: &Row<'_>) -> rusqlite::Result<RawServer> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
    ))
}

fn build_server(raw: RawServer) -> Result<McpServer> {
    let (id, tenant, name, enabled, era, created, updated, doc, sandbox) = raw;
    Ok(McpServer {
        id: id.parse().map_err(storage)?,
        tenant: TenantId::new(tenant).map_err(storage)?,
        name,
        transport: parse::<McpTransport>(&doc)?,
        enabled,
        era,
        sandbox: sandbox.as_deref().map(parse::<ServerSandbox>).transpose()?,
        created_at: Timestamp(created),
        updated_at: Timestamp(updated),
    })
}

const PIN_COLUMNS: &str = "server_id, tool, listed, status, trust, approved_hash, approved, seen_hash, seen, approved_by, approved_at, updated_at";

fn pin_from(row: &Row<'_>) -> Result<McpToolPin> {
    let get = |e: rusqlite::Error| storage(e);
    let approved: Option<String> = row.get(6).map_err(get)?;
    let seen: String = row.get(8).map_err(get)?;
    let server: String = row.get(0).map_err(get)?;
    let status: String = row.get(3).map_err(get)?;
    let trust: String = row.get(4).map_err(get)?;
    Ok(McpToolPin {
        server: server.parse().map_err(storage)?,
        tool: row.get(1).map_err(get)?,
        listed: row.get(2).map_err(get)?,
        status: PinStatus::parse(&status).ok_or_else(|| storage(format!("unknown pin status {status}")))?,
        trust: ToolTrust::parse(&trust).ok_or_else(|| storage(format!("unknown trust {trust}")))?,
        approved_hash: row.get(5).map_err(get)?,
        approved: approved.map(|t| parse::<Value>(&t)).transpose()?,
        seen_hash: row.get(7).map_err(get)?,
        seen: parse(&seen)?,
        approved_by: row.get(9).map_err(get)?,
        approved_at: row.get::<_, Option<i64>>(10).map_err(get)?.map(Timestamp),
        updated_at: Timestamp(row.get(11).map_err(get)?),
    })
}

/// The server must belong to the tenant, or it does not exist as far as the caller is concerned.
fn owns(conn: &Db, tenant: &TenantId, server: McpServerId) -> Result<()> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM mcp_servers WHERE id = ?1 AND tenant = ?2",
            params![server.to_string(), tenant.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(storage)?;
    found.map(|_| ()).ok_or_else(|| Error::Missing(format!("MCP server {server}")))
}

fn load_pins(conn: &Db, server: McpServerId) -> Result<Vec<McpToolPin>> {
    let mut statement = conn
        .prepare(&format!("SELECT {PIN_COLUMNS} FROM mcp_tool_pins WHERE server_id = ?1 ORDER BY tool"))
        .map_err(storage)?;
    let mut rows = statement.query(params![server.to_string()]).map_err(storage)?;
    let mut pins = Vec::new();
    while let Some(row) = rows.next().map_err(storage)? {
        pins.push(pin_from(row)?);
    }
    Ok(pins)
}

fn load_pin(conn: &Db, server: McpServerId, tool: &str) -> Result<McpToolPin> {
    conn.query_row(
        &format!("SELECT {PIN_COLUMNS} FROM mcp_tool_pins WHERE server_id = ?1 AND tool = ?2"),
        params![server.to_string(), tool],
        |row| Ok(pin_from(row)),
    )
    .optional()
    .map_err(storage)?
    .ok_or_else(|| Error::Missing(format!("tool {tool}")))?
}

/// True when the definition says the tool only reads and stays inside the server. Staying inside is something a server must *say*
/// (`openWorldHint: false`): the specification's default is a tool that reaches outside, so a definition with no such hint does not
/// qualify, however read-only it claims to be.
fn reports_read_only(definition: &Value) -> bool {
    let hints = &definition["annotations"];
    hints["readOnlyHint"] == true && hints["destructiveHint"] != true && hints["openWorldHint"] == false
}

const CONNECTION_COLUMNS: &str =
    "id, tenant, kind, name, settings, has_secret, created_at, updated_at, test_at, test_ok, test_note";

fn connection_from(row: &Row<'_>) -> Result<Connection> {
    let get = |e: rusqlite::Error| storage(e);
    let id: String = row.get(0).map_err(get)?;
    let tenant: String = row.get(1).map_err(get)?;
    let kind: String = row.get(2).map_err(get)?;
    let settings: String = row.get(4).map_err(get)?;
    let test_at: Option<i64> = row.get(8).map_err(get)?;
    let test_ok: Option<bool> = row.get(9).map_err(get)?;
    let test_note: Option<String> = row.get(10).map_err(get)?;
    Ok(Connection {
        id: id.parse().map_err(storage)?,
        tenant: TenantId::new(tenant).map_err(storage)?,
        kind: ConnectionKind::parse(&kind).ok_or_else(|| storage(format!("unknown connection kind {kind}")))?,
        name: row.get(3).map_err(get)?,
        settings: parse(&settings)?,
        has_secret: row.get(5).map_err(get)?,
        created_at: Timestamp(row.get(6).map_err(get)?),
        updated_at: Timestamp(row.get(7).map_err(get)?),
        last_test: match (test_at, test_ok) {
            (Some(at), Some(ok)) => Some(ConnectionTest { at: Timestamp(at), ok, note: test_note.unwrap_or_default() }),
            _ => None,
        },
    })
}

fn one_connection(conn: &Db, tenant: &TenantId, key: &str, value: &str) -> Result<Option<Connection>> {
    let sql = format!("SELECT {CONNECTION_COLUMNS} FROM connections WHERE tenant = ?1 AND {key} = ?2");
    conn.query_row(&sql, params![tenant.as_str(), value], |row| Ok(connection_from(row)))
        .optional()
        .map_err(storage)?
        .transpose()
}

#[async_trait]
impl IntegrationStore for SqliteIntegrationStore {
    async fn mcp_servers(&self, tenant: &TenantId) -> Result<Vec<McpServer>> {
        self.with(|conn| {
            let mut statement = conn
                .prepare(&format!("SELECT {SERVER_COLUMNS} FROM mcp_servers WHERE tenant = ?1 ORDER BY name"))
                .map_err(storage)?;
            let rows = statement.query_map(params![tenant.as_str()], server_from).map_err(storage)?;
            rows.map(|raw| build_server(raw.map_err(storage)?)).collect()
        })
    }

    async fn mcp_server(&self, tenant: &TenantId, id: McpServerId) -> Result<Option<McpServer>> {
        self.with(|conn| {
            conn.query_row(
                &format!("SELECT {SERVER_COLUMNS} FROM mcp_servers WHERE tenant = ?1 AND id = ?2"),
                params![tenant.as_str(), id.to_string()],
                server_from,
            )
            .optional()
            .map_err(storage)?
            .map(build_server)
            .transpose()
        })
    }

    async fn mcp_server_named(&self, tenant: &TenantId, name: &str) -> Result<Option<McpServer>> {
        self.with(|conn| {
            conn.query_row(
                &format!("SELECT {SERVER_COLUMNS} FROM mcp_servers WHERE tenant = ?1 AND name = ?2"),
                params![tenant.as_str(), name],
                server_from,
            )
            .optional()
            .map_err(storage)?
            .map(build_server)
            .transpose()
        })
    }

    async fn add_mcp_server(&self, tenant: &TenantId, new: NewMcpServer, now: Timestamp) -> Result<McpServer> {
        if !valid_name(&new.name) {
            return Err(invalid(format!(
                "“{}” is not a valid server name: use 1 to 32 lowercase letters, digits, '-' or '_', starting with a letter or digit.",
                new.name
            )));
        }
        if let McpTransport::Stdio { env, .. } = &new.transport {
            zorix_core::integrations::validate_env(env).map_err(invalid)?;
        }
        self.with(|conn| {
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM mcp_servers WHERE tenant = ?1", params![tenant.as_str()], |r| r.get(0))
                .map_err(storage)?;
            if count >= MAX_MCP_SERVERS as i64 {
                return Err(invalid(format!("There are already {MAX_MCP_SERVERS} MCP servers. Remove one first.")));
            }
            let taken: Option<i64> = conn
                .query_row("SELECT 1 FROM mcp_servers WHERE tenant = ?1 AND name = ?2", params![tenant.as_str(), new.name], |r| r.get(0))
                .optional()
                .map_err(storage)?;
            if taken.is_some() {
                return Err(invalid(format!("There is already an MCP server called {}.", new.name)));
            }
            let id = McpServerId::new();
            // Every new local server starts confined, whoever added it and however: the default is decided here, once.
            let sandbox = match &new.transport {
                McpTransport::Stdio { command, .. } => Some(ServerSandbox::for_new(command)),
                McpTransport::Http { .. } => None,
            };
            conn.execute(
                "INSERT INTO mcp_servers (id, tenant, name, enabled, era, created_at, updated_at, doc, sandbox) VALUES (?1, ?2, ?3, 0, NULL, ?4, ?4, ?5, ?6)",
                params![id.to_string(), tenant.as_str(), new.name, now.millis(), encode(&new.transport)?, sandbox.as_ref().map(encode).transpose()?],
            )
            .map_err(storage)?;
            Ok(McpServer {
                id,
                tenant: tenant.clone(),
                name: new.name,
                transport: new.transport,
                enabled: false,
                era: None,
                sandbox,
                created_at: now,
                updated_at: now,
            })
        })
    }

    async fn update_mcp_server(&self, tenant: &TenantId, server: McpServer, now: Timestamp) -> Result<McpServer> {
        if let McpTransport::Stdio { env, .. } = &server.transport {
            zorix_core::integrations::validate_env(env).map_err(invalid)?;
        }
        self.with(|conn| {
            let changed = conn
                .execute(
                    "UPDATE mcp_servers SET enabled = ?3, doc = ?4, updated_at = ?5 WHERE id = ?1 AND tenant = ?2",
                    params![
                        server.id.to_string(),
                        tenant.as_str(),
                        server.enabled,
                        encode(&server.transport)?,
                        now.millis()
                    ],
                )
                .map_err(storage)?;
            if changed == 0 {
                return Err(Error::Missing(format!("MCP server {}", server.id)));
            }
            Ok(McpServer { updated_at: now, ..server })
        })
    }

    async fn set_mcp_sandbox(
        &self,
        tenant: &TenantId,
        id: McpServerId,
        sandbox: Option<ServerSandbox>,
        now: Timestamp,
    ) -> Result<McpServer> {
        if let Some(sandbox) = &sandbox {
            sandbox.validate().map_err(invalid)?;
        }
        self.with(|conn| {
            let changed = conn
                .execute(
                    "UPDATE mcp_servers SET sandbox = ?3, updated_at = ?4 WHERE id = ?1 AND tenant = ?2",
                    params![id.to_string(), tenant.as_str(), sandbox.as_ref().map(encode).transpose()?, now.millis()],
                )
                .map_err(storage)?;
            if changed == 0 {
                return Err(Error::Missing(format!("MCP server {id}")));
            }
            conn.query_row(
                &format!("SELECT {SERVER_COLUMNS} FROM mcp_servers WHERE tenant = ?1 AND id = ?2"),
                params![tenant.as_str(), id.to_string()],
                server_from,
            )
            .map_err(storage)
            .and_then(build_server)
        })
    }

    async fn set_mcp_era(&self, tenant: &TenantId, id: McpServerId, era: Option<String>) -> Result<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE mcp_servers SET era = ?3 WHERE id = ?1 AND tenant = ?2",
                params![id.to_string(), tenant.as_str(), era],
            )
            .map_err(storage)?;
            Ok(())
        })
    }

    async fn remove_mcp_server(&self, tenant: &TenantId, id: McpServerId) -> Result<bool> {
        self.with(|conn| {
            let tx = conn.transaction().map_err(storage)?;
            let removed = tx
                .execute(
                    "DELETE FROM mcp_servers WHERE id = ?1 AND tenant = ?2",
                    params![id.to_string(), tenant.as_str()],
                )
                .map_err(storage)?
                > 0;
            if removed {
                tx.execute("DELETE FROM mcp_tool_pins WHERE server_id = ?1", params![id.to_string()])
                    .map_err(storage)?;
            }
            tx.commit().map_err(storage)?;
            Ok(removed)
        })
    }

    async fn tool_pins(&self, tenant: &TenantId, server: McpServerId) -> Result<Vec<McpToolPin>> {
        self.with(|conn| {
            owns(conn, tenant, server)?;
            load_pins(conn, server)
        })
    }

    async fn observe_tools(
        &self,
        tenant: &TenantId,
        server: McpServerId,
        observed: Vec<ObservedTool>,
        now: Timestamp,
    ) -> Result<Vec<McpToolPin>> {
        if observed.len() > MAX_TOOLS_PER_SERVER {
            return Err(invalid(format!(
                "The server lists {} tools; ZORIX keeps at most {MAX_TOOLS_PER_SERVER} per server.",
                observed.len()
            )));
        }
        self.with(|conn| {
            owns(conn, tenant, server)?;
            let tx = conn.transaction().map_err(storage)?;
            let id = server.to_string();
            tx.execute("UPDATE mcp_tool_pins SET listed = 0 WHERE server_id = ?1", params![id]).map_err(storage)?;
            for tool in &observed {
                let seen = encode(&tool.definition)?;
                let known = load_pin(&tx, server, &tool.name).ok();
                match known {
                    None => {
                        tx.execute(
                            "INSERT INTO mcp_tool_pins (server_id, tool, listed, status, trust, seen_hash, seen, updated_at) VALUES (?1, ?2, 1, 'new', 'ask', ?3, ?4, ?5)",
                            params![id, tool.name, tool.hash, seen, now.millis()],
                        )
                        .map_err(storage)?;
                    }
                    Some(pin) => {
                        // Approved and unchanged: pinned. Approved and different: changed (and no longer trusted to skip asking).
                        let status = match (pin.status, &pin.approved_hash) {
                            (PinStatus::Disabled, _) => PinStatus::Disabled,
                            (_, Some(approved)) if *approved == tool.hash => PinStatus::Pinned,
                            (_, Some(_)) => PinStatus::Changed,
                            (_, None) => PinStatus::New,
                        };
                        // A tool is only left unasked while its approved definition still qualifies: this also takes back a choice made
                        // before "inside the server" had to be said by the server.
                        let qualifies = pin.approved.as_ref().is_some_and(reports_read_only);
                        let trust = if status == PinStatus::Pinned && (pin.trust != ToolTrust::ReadOnlyNoAsk || qualifies) {
                            pin.trust
                        } else {
                            ToolTrust::Ask
                        };
                        tx.execute(
                            "UPDATE mcp_tool_pins SET listed = 1, status = ?3, trust = ?4, seen_hash = ?5, seen = ?6, updated_at = ?7 WHERE server_id = ?1 AND tool = ?2",
                            params![id, tool.name, status.as_str(), trust.as_str(), tool.hash, seen, now.millis()],
                        )
                        .map_err(storage)?;
                    }
                }
            }
            // A tool that was never approved and is gone has nothing worth keeping.
            tx.execute("DELETE FROM mcp_tool_pins WHERE server_id = ?1 AND listed = 0 AND approved_hash IS NULL", params![id]).map_err(storage)?;
            tx.commit().map_err(storage)?;
            load_pins(conn, server)
        })
    }

    async fn review_tool(
        &self,
        tenant: &TenantId,
        server: McpServerId,
        tool: &str,
        action: PinAction,
        by: &str,
        now: Timestamp,
    ) -> Result<McpToolPin> {
        if by.trim().is_empty() {
            return Err(invalid("Say who is reviewing."));
        }
        self.with(|conn| {
            owns(conn, tenant, server)?;
            let pin = load_pin(conn, server, tool)?;
            match action {
                PinAction::Approve { hash } => {
                    if !pin.listed {
                        return Err(invalid(format!("The server does not list {tool} any more, so there is nothing to approve.")));
                    }
                    if let Some(shown) = hash
                        && *shown != pin.seen_hash
                    {
                        return Err(invalid(format!(
                            "The server changed {tool} after you looked at it, so it was not approved. Read what it says now and approve again."
                        )));
                    }
                    conn.execute(
                        "UPDATE mcp_tool_pins SET status = 'pinned', approved_hash = seen_hash, approved = seen, approved_by = ?3, approved_at = ?4, updated_at = ?4 WHERE server_id = ?1 AND tool = ?2",
                        params![server.to_string(), tool, by, now.millis()],
                    )
                    .map_err(storage)?;
                }
                PinAction::Disable => {
                    conn.execute(
                        "UPDATE mcp_tool_pins SET status = 'disabled', trust = 'ask', updated_at = ?3 WHERE server_id = ?1 AND tool = ?2",
                        params![server.to_string(), tool, now.millis()],
                    )
                    .map_err(storage)?;
                }
            }
            load_pin(conn, server, tool)
        })
    }

    async fn set_tool_trust(
        &self,
        tenant: &TenantId,
        server: McpServerId,
        tool: &str,
        trust: ToolTrust,
    ) -> Result<McpToolPin> {
        self.with(|conn| {
            owns(conn, tenant, server)?;
            let pin = load_pin(conn, server, tool)?;
            if trust == ToolTrust::ReadOnlyNoAsk {
                if pin.status != PinStatus::Pinned {
                    return Err(invalid("Approve the tool's definition before choosing not to be asked."));
                }
                let approved = pin.approved.as_ref().unwrap_or(&pin.seen);
                if !reports_read_only(approved) {
                    return Err(invalid(format!(
                        "{tool} does not report itself as read-only (or it reaches outside the server), so it always asks."
                    )));
                }
            }
            conn.execute(
                "UPDATE mcp_tool_pins SET trust = ?3 WHERE server_id = ?1 AND tool = ?2",
                params![server.to_string(), tool, trust.as_str()],
            )
            .map_err(storage)?;
            load_pin(conn, server, tool)
        })
    }

    async fn connections(&self, tenant: &TenantId) -> Result<Vec<Connection>> {
        self.with(|conn| {
            let mut statement = conn
                .prepare(&format!("SELECT {CONNECTION_COLUMNS} FROM connections WHERE tenant = ?1 ORDER BY name"))
                .map_err(storage)?;
            let mut rows = statement.query(params![tenant.as_str()]).map_err(storage)?;
            let mut all = Vec::new();
            while let Some(row) = rows.next().map_err(storage)? {
                all.push(connection_from(row)?);
            }
            Ok(all)
        })
    }

    async fn connection(&self, tenant: &TenantId, id: ConnectionId) -> Result<Option<Connection>> {
        self.with(|conn| one_connection(conn, tenant, "id", &id.to_string()))
    }

    async fn connection_named(&self, tenant: &TenantId, name: &str) -> Result<Option<Connection>> {
        self.with(|conn| one_connection(conn, tenant, "name", name))
    }

    async fn add_connection(&self, tenant: &TenantId, new: NewConnection, now: Timestamp) -> Result<Connection> {
        if !valid_name(&new.name) {
            return Err(invalid(format!(
                "“{}” is not a valid connection name: use 1 to 32 lowercase letters, digits, '-' or '_', starting with a letter or digit.",
                new.name
            )));
        }
        self.with(|conn| {
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM connections WHERE tenant = ?1", params![tenant.as_str()], |r| r.get(0))
                .map_err(storage)?;
            if count >= MAX_CONNECTIONS as i64 {
                return Err(invalid(format!("There are already {MAX_CONNECTIONS} connections. Remove one first.")));
            }
            if one_connection(conn, tenant, "name", &new.name)?.is_some() {
                return Err(invalid(format!("There is already a connection called {}.", new.name)));
            }
            let id = ConnectionId::new();
            conn.execute(
                "INSERT INTO connections (id, tenant, kind, name, settings, has_secret, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![id.to_string(), tenant.as_str(), new.kind.as_str(), new.name, encode(&new.settings)?, new.has_secret, now.millis()],
            )
            .map_err(storage)?;
            Ok(Connection {
                id,
                tenant: tenant.clone(),
                kind: new.kind,
                name: new.name,
                settings: new.settings,
                has_secret: new.has_secret,
                created_at: now,
                updated_at: now,
                last_test: None,
            })
        })
    }

    async fn update_connection(
        &self,
        tenant: &TenantId,
        id: ConnectionId,
        patch: ConnectionPatch,
        now: Timestamp,
    ) -> Result<Connection> {
        self.with(|conn| {
            let current = one_connection(conn, tenant, "id", &id.to_string())?
                .ok_or_else(|| Error::Missing(format!("connection {id}")))?;
            let settings = patch.settings.unwrap_or(current.settings);
            let has_secret = patch.has_secret.unwrap_or(current.has_secret);
            conn.execute(
                "UPDATE connections SET settings = ?3, has_secret = ?4, updated_at = ?5 WHERE id = ?1 AND tenant = ?2",
                params![id.to_string(), tenant.as_str(), encode(&settings)?, has_secret, now.millis()],
            )
            .map_err(storage)?;
            one_connection(conn, tenant, "id", &id.to_string())?
                .ok_or_else(|| Error::Missing(format!("connection {id}")))
        })
    }

    async fn record_connection_test(&self, tenant: &TenantId, id: ConnectionId, test: ConnectionTest) -> Result<()> {
        self.with(|conn| {
            let changed = conn
                .execute(
                    "UPDATE connections SET test_at = ?3, test_ok = ?4, test_note = ?5 WHERE id = ?1 AND tenant = ?2",
                    params![id.to_string(), tenant.as_str(), test.at.millis(), test.ok, test.note],
                )
                .map_err(storage)?;
            if changed == 0 {
                return Err(Error::Missing(format!("connection {id}")));
            }
            Ok(())
        })
    }

    async fn remove_connection(&self, tenant: &TenantId, id: ConnectionId) -> Result<bool> {
        self.with(|conn| {
            Ok(conn
                .execute(
                    "DELETE FROM connections WHERE id = ?1 AND tenant = ?2",
                    params![id.to_string(), tenant.as_str()],
                )
                .map_err(storage)?
                > 0)
        })
    }
}
