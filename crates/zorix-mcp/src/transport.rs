//! What the client needs from a way of reaching a server.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::error::McpError;

/// The largest single message accepted from a server (ADR-0018).
pub const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
/// The most stderr text kept per server.
pub const LOG_BYTES: usize = 1024 * 1024;

/// Something a server did that is not the answer to a request.
#[derive(Clone, Debug, PartialEq)]
pub enum ServerEvent {
    /// A notification (`notifications/tools/list_changed`, progress, log messages…).
    Notification {
        /// The method.
        method: String,
        /// The parameters.
        params: Value,
    },
    /// The server asked us for something we do not give servers (sampling, elicitation, roots). It was refused.
    Refused {
        /// The method it asked for.
        method: String,
    },
    /// The connection ended.
    Closed(String),
}

/// Answers what a server asks while it works (sampling, elicitation). The error is a JSON-RPC code and message.
#[async_trait]
pub trait Responder: Send + Sync {
    /// Answer one request.
    async fn respond(&self, method: &str, params: Value) -> Result<Value, (i64, String)>;
}

/// A connection to one MCP server.
#[async_trait]
pub trait Transport: Send + Sync {
    /// Send a request and wait for its answer. Cancelling or timing out tells the server to stop and returns an error.
    async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<Value, McpError> {
        self.request_with(method, params, &[], timeout, cancel).await
    }

    /// The same with extra HTTP headers (`Mcp-Param-*`). Transports that have no headers ignore them.
    async fn request_with(
        &self,
        method: &str,
        params: Value,
        headers: &[(String, String)],
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<Value, McpError>;

    /// Send a notification.
    async fn notify(&self, method: &str, params: Value) -> Result<(), McpError>;

    /// Who answers the server's own requests from now on. Without one, they are refused.
    fn set_responder(&self, _responder: std::sync::Arc<dyn Responder>) {}

    /// Follow what the server does besides answering.
    fn subscribe(&self) -> broadcast::Receiver<ServerEvent>;

    /// False once the connection has ended.
    fn alive(&self) -> bool;

    /// What the server wrote to its error stream, newest last, with secrets removed.
    fn log(&self) -> Vec<String>;

    /// End the connection politely, then firmly.
    async fn close(&self);

    /// Wait (briefly) until the last of what the server wrote to its error stream has been read, so that [`log`](Self::log) holds its
    /// last words. A transport without such a stream returns at once.
    async fn drained(&self) {}
}
