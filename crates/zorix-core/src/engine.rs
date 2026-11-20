//! The contract clients use. The CLI, the desktop shell and later modules depend on this, not on the runtime crate.

use async_trait::async_trait;
use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use serde::{Deserialize, Serialize};

use crate::approval::{Approval, ApprovalDecision, ApprovalStatus};
use crate::error::Result;
use crate::event::ExecutionEvent;
use crate::execution::{Execution, NewExecution};
use crate::ids::{ApprovalId, ExecutionId, TenantId};
use crate::repository::{ListFilter, ToolCallRecord};
use crate::state::ExecutionState;

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

/// An approval that waits in a tree, with who asked.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TreeApproval {
    /// The execution that is waiting (a child, a grandchild or the root itself).
    pub execution: ExecutionId,
    /// The agent that run belongs to, when an agent made it.
    pub agent: Option<String>,
    /// How deep it is in the tree.
    pub depth: u32,
    /// The approval, to be decided on `execution`.
    pub approval: Approval,
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
    /// How many looked interrupted but are being worked on by a live engine (their lease is held), so were skipped.
    #[serde(default)]
    pub in_use: usize,
}

/// A piece of an answer that is still being written. Live events are never stored: the `model.responded` event is the
/// record, and it replaces what the pieces showed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LiveEvent {
    /// The next piece of the text.
    Text {
        /// The execution that is writing.
        execution: ExecutionId,
        /// The piece.
        delta: String,
    },
    /// Forget the pieces so far: another model is taking over.
    Reset {
        /// The execution that is writing.
        execution: ExecutionId,
    },
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

    /// An execution and everything it started, nearest first (the execution itself comes first).
    async fn tree(&self, tenant: &TenantId, root: ExecutionId) -> Result<Vec<Execution>> {
        let top = self.get(tenant, root).await?;
        let mut all = vec![top];
        let mut frontier = vec![root];
        while let Some(next) = frontier.pop() {
            let filter = ListFilter { parent: Some(next), limit: 1_000, ..ListFilter::default() };
            let mut children = self.list(tenant, &filter).await?;
            children.sort_by_key(|e| (e.created_at, e.id));
            for child in children {
                if !all.iter().any(|e| e.id == child.id) {
                    frontier.insert(0, child.id);
                    all.push(child);
                }
            }
            if all.len() > 10_000 {
                break;
            }
        }
        Ok(all)
    }

    /// The approvals waiting for a person anywhere in the tree of `root`, each with the execution that asked.
    async fn pending_in_tree(&self, tenant: &TenantId, root: ExecutionId) -> Result<Vec<TreeApproval>> {
        let mut out = Vec::new();
        for execution in self.tree(tenant, root).await? {
            if execution.state != ExecutionState::WaitingForApproval {
                continue;
            }
            let detail = self.detail(tenant, execution.id).await?;
            for approval in detail.approvals.into_iter().filter(|a| a.status == ApprovalStatus::Pending) {
                out.push(TreeApproval {
                    execution: execution.id,
                    agent: execution.agent.as_ref().map(|a| a.slug.to_string()),
                    depth: execution.depth,
                    approval,
                });
            }
        }
        Ok(out)
    }

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

    /// The text of this execution's answers as it is written, for a screen that shows it before the step is stored.
    /// Pieces are not kept: someone who connects late sees only what comes next. The stream never ends by itself; drop it
    /// when the execution is over. An engine that cannot stream gives an empty stream.
    async fn live(&self, tenant: &TenantId, id: ExecutionId) -> Result<BoxStream<'static, LiveEvent>> {
        self.get(tenant, id).await?;
        Ok(futures_util::stream::pending().boxed())
    }

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
