//! ZORIX's Model Context Protocol client (ADR-0018): stdio and HTTP transports, both protocol eras, and the trust layer
//! that pins what a server's tools say.

pub mod auth;
pub mod catalog;
pub mod client;
pub mod error;
pub mod headers;
pub mod http;
pub mod import;
pub mod manager;
pub mod oauth;
pub mod protocol;
pub mod server;
pub mod stdio;
pub mod tool;
pub mod transport;
pub mod wire;

pub use auth::{AuthProvider, NoAuth, StaticBearer};
pub use catalog::{Catalog, CatalogEntry, catalog};
pub use client::{CallOutput, McpClient, McpToolDef, ServerInfo, ToolListing, valid_tool_name};
pub use error::McpError;
pub use http::HttpTransport;
pub use manager::{
    Connector, DefaultConnector, ManagerOptions, McpManager, ServerState, ServerStatus, StatusChange, env_secret_name,
    secret_base, token_secret_name,
};
pub use oauth::{BrowserOpener, LoginSummary, OAuthClient, OAuthSession, OAuthSettings, SystemBrowser, Vault};
pub use protocol::{Era, LEGACY_VERSIONS, MODERN_VERSION};
pub use server::{MAX_IN_FLIGHT, MAX_REQUEST_BYTES, McpServer, ServerConfig};
pub use stdio::{StdioConfig, StdioTransport};
pub use tool::{McpCaller, McpTool, tool_id};
pub use transport::{ServerEvent, Transport};
