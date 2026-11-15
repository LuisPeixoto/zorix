//! Building the context for a model call (ADR-0007).

use async_trait::async_trait;

use crate::error::Result;
use crate::execution::Execution;
use crate::message::Message;
use crate::model::ModelRequest;
use crate::tool::ToolSpec;

/// What a context source can look at.
#[derive(Clone, Copy, Debug)]
pub struct ExecutionView<'a> {
    /// The execution.
    pub execution: &'a Execution,
    /// What was said so far in the loop (assistant messages and tool results).
    pub transcript: &'a [Message],
    /// The tools this execution may use.
    pub tools: &'a [ToolSpec],
    /// How many tokens the model can be sent, from its profile: the window minus room for the answer and a margin.
    /// `None` when nothing is known about the model.
    pub context_budget: Option<u64>,
}

/// How important a segment is when the budget is short. Higher survives longer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Priority(pub u8);

impl Priority {
    /// Dropped first.
    pub const LOW: Priority = Priority(10);
    /// Normal.
    pub const NORMAL: Priority = Priority(50);
    /// Dropped last.
    pub const HIGH: Priority = Priority(90);
}

/// A piece of context.
#[derive(Clone, Debug, PartialEq)]
pub struct ContextSegment {
    /// Where it came from (for the record of what was left out).
    pub origin: String,
    /// The messages.
    pub messages: Vec<Message>,
    /// How much it matters.
    pub priority: Priority,
    /// Never dropped. The budget is exceeded rather than losing it.
    pub required: bool,
}

/// Something that contributes context: instructions, history, and later memory or retrieved knowledge.
#[async_trait]
pub trait ContextSource: Send + Sync {
    /// A short name, such as `history`.
    fn name(&self) -> &str;

    /// The segments to add, in the order they should appear.
    async fn contribute(&self, view: &ExecutionView<'_>) -> Result<Vec<ContextSegment>>;
}

/// Turns an execution into a model request.
#[async_trait]
pub trait ContextBuilder: Send + Sync {
    /// Build the request for the next model call.
    async fn build(&self, view: &ExecutionView<'_>) -> Result<ModelRequest>;
}
