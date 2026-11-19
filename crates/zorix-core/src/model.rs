//! The contract with whatever produces model answers. The runtime never knows which provider is behind it.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::execution::Usage;
use crate::ids::ExecutionId;
use crate::message::{Message, NativeTurn, ToolCallRequest};
use crate::tool::ToolDefinition;

/// How hard the model should think. Providers that have no such control ignore it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    /// Fastest and cheapest.
    Low,
    /// A balance.
    Medium,
    /// Thorough. A good default for tool loops.
    High,
    /// Very thorough.
    XHigh,
    /// As thorough as the model gets.
    Max,
}

/// Settings for one model call that are not part of the conversation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GenerationParams {
    /// Most tokens the model may write. `None` lets the adapter choose.
    pub max_output_tokens: Option<u32>,
    /// How hard to think. `None` lets the adapter choose.
    pub effort: Option<Effort>,
}

/// Where in an execution a call happens. A route may use different models for different phases.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepPhase {
    /// The first call: understanding the request.
    #[default]
    Start,
    /// A later call, after tool results.
    Continue,
    /// The call is repeated after the answer was rejected.
    Retry,
}

/// What is sent to a model.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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
    /// Output size and effort.
    #[serde(default)]
    pub params: GenerationParams,
    /// Where in the execution this call is.
    #[serde(default)]
    pub phase: StepPhase,
    /// Earlier turns were left out to fit the context window. Adapters then send the portable form of the conversation,
    /// without native turns, because a provider may refuse reasoning state whose prefix changed.
    #[serde(default)]
    pub history_edited: bool,
    /// What each context segment contributed and whether it was kept. For events; never sent to a provider.
    #[serde(default, skip)]
    pub context_report: Vec<SegmentReport>,
    /// The models this call may use, when an agent made the run: patterns of `provider/model`, `provider/*`, `demo` or `*`
    /// (ADR-0027). Empty means any. The router drops every candidate outside it, fallbacks included. Never sent to a provider.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_models: Vec<String>,
}

impl ModelRequest {
    /// A rough size of the tool definitions, in tokens (characters divided by four, plus a little per tool).
    pub fn estimated_tool_tokens(&self) -> u64 {
        self.tools.iter().map(|t| (t.description.len() + t.input_schema.to_string().len()) as u64 / 4 + 8).sum()
    }
}

/// How one context segment fared when the context was built.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentReport {
    /// Where it came from (`system`, `long_term`, `knowledge`, `history`…).
    pub origin: String,
    /// Its size in calibrated tokens.
    pub tokens: u64,
    /// False when it was left out to fit the budget.
    pub kept: bool,
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
    /// The provider declined to answer.
    Refusal,
}

/// One candidate a router tried.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteAttempt {
    /// The provider.
    pub provider: String,
    /// The model.
    pub model: String,
    /// `ok`, or why it was passed over (`rate_limited`, `unavailable`, `skipped: no tool support`…).
    pub outcome: String,
}

/// What a router did to answer one call.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteTrace {
    /// The candidates tried, in order. The last one answered.
    pub attempts: Vec<RouteAttempt>,
}

impl RouteTrace {
    /// True when something other than the first candidate answered.
    pub fn fell_back(&self) -> bool {
        self.attempts.len() > 1
    }
}

/// What a model answered.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelResponse {
    /// The model that answered (it may differ from the one asked for, when a gateway routes).
    pub model: String,
    /// The provider that answered, when there is one.
    #[serde(default)]
    pub provider: Option<String>,
    /// The text. May be empty when the model only asks for tools.
    pub text: String,
    /// The tools asked for, in order. Empty means the text is the final answer.
    pub tool_calls: Vec<ToolCallRequest>,
    /// The provider's raw turn, to be replayed unchanged to the same model.
    #[serde(default)]
    pub native: Option<NativeTurn>,
    /// What the call consumed.
    pub usage: Usage,
    /// Why it stopped.
    pub finish: FinishReason,
    /// What a router did, when one was involved.
    #[serde(default)]
    pub route: Option<RouteTrace>,
}

impl ModelResponse {
    /// A response with no provider, native turn or route.
    pub fn new(
        model: impl Into<String>,
        text: String,
        tool_calls: Vec<ToolCallRequest>,
        usage: Usage,
        finish: FinishReason,
    ) -> Self {
        Self { model: model.into(), provider: None, text, tool_calls, native: None, usage, finish, route: None }
    }
}

/// What kind of failure a model call had. Routers decide what to do from this.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelErrorKind {
    /// The credentials are missing or wrong.
    Auth,
    /// The model or the endpoint does not exist (or is not available to this account).
    NotFound,
    /// The request does not fit the model's context window.
    ContextTooLong,
    /// Too many requests.
    RateLimited,
    /// The provider or the network failed, or is overloaded.
    Unavailable,
    /// The provider declined to answer.
    Refused,
    /// The request is wrong and cannot succeed as it is.
    Invalid,
    /// The call was cancelled.
    Cancelled,
    /// This provider or model cannot do what was asked (for example, Anthropic has no embeddings).
    Unsupported,
}

impl ModelErrorKind {
    /// True when trying the same call again, a little later, may work.
    pub fn is_temporary(&self) -> bool {
        matches!(self, ModelErrorKind::RateLimited | ModelErrorKind::Unavailable)
    }

    /// True when another candidate model or provider may succeed where this one did not.
    pub fn allows_fallback(&self) -> bool {
        !matches!(self, ModelErrorKind::Invalid | ModelErrorKind::Cancelled)
    }

    /// The name used in events and traces.
    pub fn as_str(&self) -> &'static str {
        match self {
            ModelErrorKind::Auth => "auth",
            ModelErrorKind::NotFound => "not_found",
            ModelErrorKind::ContextTooLong => "context_too_long",
            ModelErrorKind::RateLimited => "rate_limited",
            ModelErrorKind::Unavailable => "unavailable",
            ModelErrorKind::Refused => "refused",
            ModelErrorKind::Invalid => "invalid",
            ModelErrorKind::Cancelled => "cancelled",
            ModelErrorKind::Unsupported => "unsupported",
        }
    }
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
    /// A failure from a real provider, with its kind.
    #[error("{message}")]
    Provider {
        /// What kind of failure.
        kind: ModelErrorKind,
        /// A sentence for a person. Never holds a secret.
        message: String,
        /// How long the provider asked us to wait, in milliseconds.
        retry_after_ms: Option<u64>,
    },
    /// The call was cancelled.
    #[error("The model call was cancelled.")]
    Cancelled,
}

impl ModelError {
    /// A provider failure.
    pub fn provider(kind: ModelErrorKind, message: impl Into<String>) -> Self {
        ModelError::Provider { kind, message: message.into(), retry_after_ms: None }
    }

    /// What kind of failure this is. The older variants map to the nearest kind.
    pub fn kind(&self) -> ModelErrorKind {
        match self {
            ModelError::Transient { .. } => ModelErrorKind::Unavailable,
            ModelError::Permanent(_) => ModelErrorKind::Invalid,
            ModelError::Provider { kind, .. } => *kind,
            ModelError::Cancelled => ModelErrorKind::Cancelled,
        }
    }
}

/// What a gateway knows about a model it serves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelProfile {
    /// The context window, in tokens.
    pub context_window: u64,
    /// The most tokens it can write in one answer.
    pub max_output: u64,
    /// It can ask for tools.
    pub tools: bool,
    /// It can read images.
    pub vision: bool,
}

/// Where the text of an answer goes while it is still being written. Nothing sent here is stored: the finished
/// [`ModelResponse`] is the record, and it replaces whatever a sink showed.
pub trait TextSink: Send + Sync {
    /// The next piece of the answer.
    fn text(&self, delta: &str);

    /// Forget what was sent for this call: a router is about to try another model.
    fn reset(&self) {}
}

/// A sink that drops everything.
pub struct NoSink;

impl TextSink for NoSink {
    fn text(&self, _delta: &str) {}
}

/// Something that turns a [`ModelRequest`] into a [`ModelResponse`].
#[async_trait]
pub trait ModelGateway: Send + Sync {
    /// Make one call. It must stop early when `cancel` fires.
    async fn generate(&self, request: ModelRequest, cancel: CancellationToken) -> Result<ModelResponse, ModelError>;

    /// Like [`generate`](Self::generate), and the text is also passed to `sink` as it arrives. A gateway that cannot
    /// stream answers once the call is done, with the whole text in a single piece.
    async fn generate_live(
        &self,
        request: ModelRequest,
        cancel: CancellationToken,
        sink: std::sync::Arc<dyn TextSink>,
    ) -> Result<ModelResponse, ModelError> {
        let response = self.generate(request, cancel).await?;
        if !response.text.is_empty() {
            sink.text(&response.text);
        }
        Ok(response)
    }

    /// What is known about `model`, for sizing the context. `None` means nothing is known.
    fn describe(&self, _model: &str) -> Option<ModelProfile> {
        None
    }

    /// Turn texts into vectors. Providers without embeddings answer `Unsupported`, which is the default.
    async fn embed(
        &self,
        _request: EmbeddingRequest,
        _cancel: CancellationToken,
    ) -> Result<EmbeddingResponse, ModelError> {
        Err(ModelError::provider(ModelErrorKind::Unsupported, "This provider has no embeddings."))
    }

    /// What is known about an embeddings model, for sizing batches. `None` means nothing is known.
    fn embedding_profile(&self, _model: &str) -> Option<EmbeddingProfile> {
        None
    }
}

/// Why a text is being embedded. Some providers (and some models, through a prefix) treat the two differently.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbedPurpose {
    /// A piece of a document or a memory, to be searched.
    Document,
    /// A question, to search with.
    Query,
}

/// Texts to turn into vectors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingRequest {
    /// The model, as the provider names it.
    pub model: String,
    /// The texts. One vector comes back per text, in the same order.
    pub inputs: Vec<String>,
    /// Document or query.
    pub purpose: EmbedPurpose,
    /// The size of the vectors, for models that allow choosing.
    pub dimensions: Option<u32>,
}

/// The vectors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingResponse {
    /// The model that answered.
    pub model: String,
    /// The provider that answered, when known.
    pub provider: Option<String>,
    /// One L2-normalised vector per input.
    pub vectors: Vec<Vec<f32>>,
    /// What the call used.
    pub usage: Usage,
}

/// What a gateway knows about an embeddings model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddingProfile {
    /// The size of its vectors.
    pub dimensions: u32,
    /// The longest input, in tokens.
    pub max_input_tokens: u64,
    /// The most inputs in one call.
    pub max_batch: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_older_error_variants_map_to_kinds() {
        assert_eq!(
            ModelError::Transient { message: "x".into(), retry_after_ms: None }.kind(),
            ModelErrorKind::Unavailable
        );
        assert_eq!(ModelError::Permanent("x".into()).kind(), ModelErrorKind::Invalid);
        assert_eq!(ModelError::Cancelled.kind(), ModelErrorKind::Cancelled);
        assert_eq!(ModelError::provider(ModelErrorKind::Auth, "no key").kind(), ModelErrorKind::Auth);
    }

    #[test]
    fn what_may_be_retried_and_what_may_fall_back() {
        use ModelErrorKind::*;
        for kind in [RateLimited, Unavailable] {
            assert!(kind.is_temporary() && kind.allows_fallback(), "{kind:?}");
        }
        for kind in [Auth, NotFound, ContextTooLong, Refused] {
            assert!(!kind.is_temporary() && kind.allows_fallback(), "{kind:?}");
        }
        for kind in [Invalid, Cancelled] {
            assert!(!kind.allows_fallback(), "{kind:?}");
        }
    }

    #[test]
    fn a_request_from_version_0_1_loads_with_defaults() {
        let old = format!(r#"{{"model":"m","execution_id":"{}","messages":[],"tools":[]}}"#, ExecutionId::new());
        let request: ModelRequest = serde_json::from_str(&old).unwrap();
        assert_eq!(
            (request.phase, request.history_edited, request.params),
            (StepPhase::Start, false, GenerationParams::default())
        );
    }

    #[test]
    fn a_trace_knows_when_it_fell_back() {
        let attempt = |outcome: &str| RouteAttempt { provider: "p".into(), model: "m".into(), outcome: outcome.into() };
        assert!(!RouteTrace { attempts: vec![attempt("ok")] }.fell_back());
        assert!(RouteTrace { attempts: vec![attempt("rate_limited"), attempt("ok")] }.fell_back());
    }
}
