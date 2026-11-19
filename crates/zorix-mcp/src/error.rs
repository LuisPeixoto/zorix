//! What can go wrong talking to an MCP server, in words a person can act on.

use serde_json::Value;

/// A failure of an MCP call or connection.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum McpError {
    /// The server program could not be started.
    #[error("The server could not be started: {0}")]
    Spawn(String),
    /// Reading or writing to the server failed.
    #[error("Talking to the server failed: {0}")]
    Transport(String),
    /// The server did not answer in time.
    #[error("The server did not answer in time: {0}")]
    Timeout(String),
    /// The call was cancelled.
    #[error("The call was cancelled.")]
    Cancelled,
    /// The server answered something that is not valid MCP.
    #[error("The server broke the protocol: {0}")]
    Protocol(String),
    /// The server answered with a JSON-RPC error.
    #[error("The server returned an error ({code}): {message}")]
    Rpc {
        /// The JSON-RPC code.
        code: i64,
        /// What the server said.
        message: String,
        /// Extra data, when it sent some.
        data: Option<Value>,
    },
    /// The server needs a person to sign in.
    #[error("The server needs you to sign in: {0}")]
    NeedsLogin(String),
    /// The server asked for something ZORIX does not give servers (more input, model access…).
    #[error("{0}")]
    Refused(String),
    /// A message was bigger than the limit.
    #[error("The server sent a message larger than {limit} bytes, so the connection was closed.")]
    TooLarge {
        /// The limit.
        limit: usize,
    },
    /// The connection is closed.
    #[error("The server is not running.")]
    Closed,
}

impl McpError {
    /// True when asking again may work (the server may just have been slow or restarted).
    pub fn is_retryable(&self) -> bool {
        matches!(self, McpError::Timeout(_) | McpError::Closed | McpError::Transport(_))
    }
}

/// The JSON-RPC code of "method not found".
pub const METHOD_NOT_FOUND: i64 = -32601;
/// The JSON-RPC code of "unsupported protocol version" in the 2026-07-28 revision.
pub const UNSUPPORTED_VERSION: i64 = -32022;
