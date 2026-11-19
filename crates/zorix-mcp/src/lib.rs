//! ZORIX's Model Context Protocol client (ADR-0018): stdio and HTTP transports, both protocol eras, and the trust layer
//! that pins what a server's tools say.

pub mod client;
pub mod error;
pub mod protocol;
pub mod stdio;
pub mod transport;
pub mod wire;

pub use client::{CallOutput, McpClient, McpToolDef, ServerInfo, ToolListing, valid_tool_name};
pub use error::McpError;
pub use protocol::{Era, LEGACY_VERSIONS, MODERN_VERSION};
pub use stdio::{StdioConfig, StdioTransport};
pub use transport::{ServerEvent, Transport};
