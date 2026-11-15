//! The contract clients use. The CLI, the desktop shell and later modules depend on this, not on the runtime crate.

use async_trait::async_trait;
use futures_util::stream::BoxStream;
use serde::{Deserialize, Serialize};

use crate::approval::{Approval, ApprovalDecision};
use crate::error::Result;
use crate::event::ExecutionEvent;
use crate::execution::{Execution, NewExecution};
use crate::ids::{ApprovalId, ExecutionId, TenantId};
use crate::repository::{ListFilter, ToolCallRecord};

/// An execution with the records that belong to it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExecutionDetail {
    /// The execution.
    pub execution: Execution,
    /// Its tool calls.
    pub tool_calls: Vec<ToolCallRecord>,
    /// Its approvals.
    pub approvals: Vec<Approval>,
}

/// What recovery did.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryReport {
    /// Executions put back in the queue to continue from their checkpoint.
    pub requeued: Vec<ExecutionId>,
    /// Executions whose last tool call may or may not have happened, now waiting for a person's decision.
    pub needs_decision: Vec<ExecutionId>,
    /// How many executions were left as they were (waiting for approval, paused, queued).
    pub untouched: usize,
}

/// Starts, follows and controls executions.
#[async_trait]
pub trait ExecutionEngine: Send + Sync {
    /// Validate and store a new execution (or return the one an idempotency key already made).
    async fn create(&self, new: NewExecution) -> Result<Execution>;

    /// Ask for a created execution to run.
    async fn start(&self, tenant: &TenantId, id: ExecutionId) -> Result<Execution>;

    /// One execution.
    async fn get(&self, tenant: &TenantId, id: ExecutionId) -> Result<Execution>;

    /// An execution with its tool calls and approvals.
    async fn detail(&self, tenant: &TenantId, id: ExecutionId) -> Result<ExecutionDetail>;

    /// Executions of a tenant, newest first.
    async fn list(&self, tenant: &TenantId, filter: &ListFilter) -> Result<Vec<Execution>>;

    /// Stored events after `after_seq`.
    async fn events(&self, tenant: &TenantId, id: ExecutionId, after_seq: u64) -> Result<Vec<ExecutionEvent>>;

    /// Events after `after_seq` (replayed from storage) and then live ones, without gaps or repeats.
    /// The stream ends when the execution reaches a terminal state.
    async fn subscribe(
        &self,
        tenant: &TenantId,
        id: ExecutionId,
        after_seq: u64,
    ) -> Result<BoxStream<'static, ExecutionEvent>>;

    /// Wait until the execution is finished, waiting for approval or paused, and return it.
    async fn settle(&self, tenant: &TenantId, id: ExecutionId) -> Result<Execution>;

    /// Pause at the next step boundary.
    async fn pause(&self, tenant: &TenantId, id: ExecutionId) -> Result<Execution>;

    /// Continue a paused execution.
    async fn resume(&self, tenant: &TenantId, id: ExecutionId) -> Result<Execution>;

    /// Stop for good.
    async fn cancel(&self, tenant: &TenantId, id: ExecutionId, reason: Option<String>) -> Result<Execution>;

    /// Answer a pending approval. Only this method can resolve one.
    async fn decide(
        &self,
        tenant: &TenantId,
        id: ExecutionId,
        approval: ApprovalId,
        decision: ApprovalDecision,
    ) -> Result<Execution>;

    /// Put interrupted executions back in motion. Call once when the process starts.
    async fn recover(&self) -> Result<RecoveryReport>;
}
