//! The contract with whatever produces model answers. The runtime never knows which provider is behind it.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::execution::Usage;
use crate::ids::ExecutionId;
use crate::message::{Message, ToolCallRequest};
use crate::tool::ToolDefinition;

/// What is sent to a model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelRequest {
    /// The model, as the request named it.
    pub model: String,
    /// The execution asking (for tracing and for simulators).
    pub execution_id: ExecutionId,
    /// The conversation so far.
    pub messages: Vec<Message>,
    /// The tools the model may ask for.
    pub tools: Vec<ToolDefinition>,
    /// A JSON Schema the answer should follow, when it is structured.
    #[serde(default)]
    pub response_schema: Option<Value>,
}

/// Why the model stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    /// A normal end of answer.
    Stop,
    /// The model wants tools to run.
    ToolCalls,
    /// The model ran out of room.
    Length,
}

/// What a model answered.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelResponse {
    /// The model that answered (it may differ from the one asked for, when a gateway routes).
    pub model: String,
    /// The text. May be empty when the model only asks for tools.
    pub text: String,
    /// The tools asked for, in order. Empty means the text is the final answer.
    pub tool_calls: Vec<ToolCallRequest>,
    /// What the call consumed.
    pub usage: Usage,
    /// Why it stopped.
    pub finish: FinishReason,
}

/// Why a model call failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    /// A temporary problem (rate limit, timeout, network); trying again may work.
    #[error("{message}")]
    Transient {
        /// What happened.
        message: String,
        /// How long the provider asked us to wait, in milliseconds.
        retry_after_ms: Option<u64>,
    },
    /// The call cannot succeed as it is (bad credentials, unknown model, refused content).
    #[error("{0}")]
    Permanent(String),
    /// The call was cancelled.
    #[error("The model call was cancelled.")]
    Cancelled,
}

/// Something that turns a [`ModelRequest`] into a [`ModelResponse`].
#[async_trait]
pub trait ModelGateway: Send + Sync {
    /// Make one call. It must stop early when `cancel` fires.
    async fn generate(&self, request: ModelRequest, cancel: CancellationToken) -> Result<ModelResponse, ModelError>;
}
