//! Small contracts at the edges of the engine: publishing events, scheduling work and validating results.

use async_trait::async_trait;

use crate::error::Result;
use crate::event::ExecutionEvent;
use crate::execution::{Execution, ExecutionResult};
use crate::ids::ExecutionId;

/// Delivers committed events to whoever is listening. Called after the repository has stored them.
pub trait EventPublisher: Send + Sync {
    /// Publish events in order. Must not block and must not fail the execution.
    fn publish(&self, events: &[ExecutionEvent]);
}

/// Hands queued executions to workers, one worker per execution.
#[async_trait]
pub trait ExecutionScheduler: Send + Sync {
    /// Ask for the execution to be worked on. Enqueuing one that is already queued has no extra effect.
    async fn enqueue(&self, id: ExecutionId) -> Result<()>;

    /// Wait for the next execution to work on. `None` means the scheduler was closed.
    async fn next(&self) -> Option<ExecutionId>;

    /// Stop handing out work and wake every waiting worker.
    fn close(&self);
}

/// The verdict on a final answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Validation {
    /// Good enough.
    Accept,
    /// Not good enough. The reason is sent back to the model.
    Reject {
        /// What is wrong, in a sentence.
        reason: String,
    },
}

/// Checks a final answer before the execution completes.
#[async_trait]
pub trait ResultValidator: Send + Sync {
    /// A short name for events and logs.
    fn name(&self) -> &str;

    /// Judge an answer.
    async fn validate(&self, execution: &Execution, result: &ExecutionResult) -> Validation;
}
