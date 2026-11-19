//! ZORIX's native integrations (ADR-0021). Each tool is behind grants on the resource it touches and asks a person before it
//! changes anything. Secrets live in the keychain and are chosen by the tool, never taken from the model.

pub mod api;
pub mod connections;
pub mod discord;
pub mod forge;
pub mod http_tools;
pub mod jira;
pub mod net;
pub mod slack;

use zorix_core::ConnectionId;

pub use connections::Services;
pub use discord::discord_tools;
pub use forge::forge_tools;
pub use http_tools::{http_tools, signature};
pub use jira::jira_tools;
pub use slack::slack_tools;

/// The keychain entry that holds the secret of a connection (a token, a password, a signing key).
pub fn secret_name(id: ConnectionId) -> String {
    let id = id.to_string().replace('-', "");
    format!("conn.{}", &id[id.len() - 12..])
}
