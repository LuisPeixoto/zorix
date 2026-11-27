//! Persistence contract (ADR-0004).
//!
//! The repository is deliberately coarse: [`ExecutionRepository::commit`] writes the execution, its new events, a
//! checkpoint, and the tool call and approval records in **one atomic step**, so a crash can never leave a state change
//! without its event or checkpoint.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::approval::Approval;
use crate::error::Result;
use crate::event::{ExecutionEvent, NewEvent};
use crate::execution::Execution;
use crate::ids::{ApprovalId, ConversationId, ExecutionId, TenantId, ToolCallId};
use crate::message::{Message, ToolCallRequest};
use crate::state::ExecutionState;
use crate::time::Timestamp;

/// Everything the loop needs to continue where it stopped.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LoopState {
    /// What was said in the loop so far: assistant messages and tool results.
    pub transcript: Vec<Message>,
    /// Tool calls from the last assistant message that have not been settled yet, in order.
    pub pending: Vec<ToolCallRequest>,
    /// A summary that stands for the first `covers` messages of the transcript, once they stopped fitting the model's window. The
    /// transcript itself is never cut: this is only what the model is shown instead of those messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<HistorySummary>,
}

/// What stands for the oldest part of a conversation that no longer fits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistorySummary {
    /// How many messages of the transcript, from the start, it replaces. Always the end of a turn, so a tool call is never separated from its result.
    pub covers: usize,
    /// The summary, written by a model from the messages it replaces (and the summary before it). Untrusted: it comes from tool output.
    pub text: String,
}

/// A saved [`LoopState`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Checkpoint {
    /// The execution.
    pub execution_id: ExecutionId,
    /// Counts up with every checkpoint of the execution.
    pub step: u64,
    /// The state to continue from.
    pub state: LoopState,
    /// When it was saved.
    pub created_at: Timestamp,
}

/// Where a tool call stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    /// The model asked; nothing decided yet.
    Requested,
    /// Waiting for a person.
    AwaitingApproval,
    /// Running (or was running when the process stopped).
    Started,
    /// Finished with a result.
    Succeeded,
    /// Finished with an error.
    Failed,
    /// Refused by the policy or by a person.
    Denied,
    /// Handed to other executions, which are still working; the result arrives when they finish (ADR-0029).
    Delegated,
}

impl ToolCallStatus {
    /// True when the call needs nothing more.
    pub fn is_settled(&self) -> bool {
        matches!(self, ToolCallStatus::Succeeded | ToolCallStatus::Failed | ToolCallStatus::Denied)
    }
}

/// The record of one tool call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCallRecord {
    /// The execution.
    pub execution_id: ExecutionId,
    /// The call.
    pub call_id: ToolCallId,
    /// The tool.
    pub tool: String,
    /// The arguments as the model sent them.
    pub arguments: Value,
    /// `call_hash(tool, arguments)`.
    pub call_hash: String,
    /// Where it stands.
    pub status: ToolCallStatus,
    /// The result, once succeeded (already cut to the output limit).
    pub result: Option<Value>,
    /// The error text, once failed or denied.
    pub error: Option<String>,
    /// How many times it was started.
    pub attempts: u32,
    /// Whether the tool declared itself safe to repeat.
    pub idempotent: bool,
    /// When the model asked.
    pub requested_at: Timestamp,
    /// When it last started.
    pub started_at: Option<Timestamp>,
    /// When it settled.
    pub finished_at: Option<Timestamp>,
}

/// One atomic change.
#[derive(Clone, Debug, PartialEq)]
pub struct Commit {
    /// The execution as it should be stored. Its `version` is ignored: the repository sets `expected_version + 1`.
    pub execution: Execution,
    /// The version the caller read. The commit fails with `Conflict` if the stored one differs.
    pub expected_version: u64,
    /// Events to append, in order.
    pub events: Vec<NewEvent>,
    /// A checkpoint to save.
    pub checkpoint: Option<Checkpoint>,
    /// Tool call records to insert or replace.
    pub tool_calls: Vec<ToolCallRecord>,
    /// Approvals to insert or replace.
    pub approvals: Vec<Approval>,
    /// The engine that makes this commit while it drives the execution. The commit fails with `Conflict` unless that engine still holds
    /// the execution's lease, so an engine that lost it (another took it over) can no longer mark a tool as started or write anything
    /// else. `None` for a change made from outside a run (a cancel, an answer to an approval).
    pub lease_owner: Option<String>,
}

impl Commit {
    /// A commit of `execution` that was read at `expected_version`.
    pub fn new(execution: Execution, expected_version: u64) -> Self {
        Self {
            execution,
            expected_version,
            events: Vec::new(),
            checkpoint: None,
            tool_calls: Vec::new(),
            approvals: Vec::new(),
            lease_owner: None,
        }
    }
}

/// What a commit produced.
#[derive(Clone, Debug, PartialEq)]
pub struct Committed {
    /// The execution as stored, with its new version.
    pub execution: Execution,
    /// The events as stored, numbered.
    pub events: Vec<ExecutionEvent>,
}

/// What creating an execution did.
#[derive(Clone, Debug, PartialEq)]
pub enum CreateOutcome {
    /// A new execution was stored.
    Created(Committed),
    /// The idempotency key was already used; this is the execution it made.
    Existing(Execution),
}

/// Which executions to list.
#[derive(Clone, Debug, Default)]
pub struct ListFilter {
    /// Only these states. Empty means all.
    pub states: Vec<ExecutionState>,
    /// Only this conversation.
    pub conversation: Option<ConversationId>,
    /// Only runs of this agent.
    pub agent: Option<crate::ids::AgentId>,
    /// Only the executions this one started.
    pub parent: Option<crate::ids::ExecutionId>,
    /// Only executions nobody started (the tops of trees).
    pub roots_only: bool,
    /// Only executions created at or after this time.
    pub since: Option<Timestamp>,
    /// At most this many (newest first). Zero means the default of 100.
    pub limit: usize,
}

/// Stores executions and everything that belongs to them.
///
/// Every read of a single execution takes the tenant: an execution of another tenant is reported as not found.
#[async_trait]
pub trait ExecutionRepository: Send + Sync {
    /// Store a new execution with its first events. If `execution.idempotency_key` was used before for the same
    /// tenant, nothing is stored and the earlier execution is returned.
    async fn create(&self, execution: Execution, events: Vec<NewEvent>) -> Result<CreateOutcome>;

    /// Apply a change atomically. Fails with `Error::Conflict` when the version does not match.
    async fn commit(&self, commit: Commit) -> Result<Committed>;

    /// One execution by id, whoever owns it. For the runtime's own workers, which only hold an id; interfaces must use
    /// [`get`](Self::get) so tenants stay apart.
    async fn load(&self, id: ExecutionId) -> Result<Option<Execution>>;

    /// One execution of this tenant.
    async fn get(&self, tenant: &TenantId, id: ExecutionId) -> Result<Option<Execution>>;

    /// Executions of this tenant, newest first.
    async fn list(&self, tenant: &TenantId, filter: &ListFilter) -> Result<Vec<Execution>>;

    /// The executions one execution started, oldest first.
    async fn children(&self, tenant: &TenantId, parent: ExecutionId) -> Result<Vec<Execution>> {
        let filter = ListFilter { parent: Some(parent), limit: 1_000, ..ListFilter::default() };
        let mut found = self.list(tenant, &filter).await?;
        found.sort_by_key(|e| (e.created_at, e.id));
        Ok(found)
    }

    /// Every execution below `root` (children, their children, and so on), nearest first. The root itself is not included.
    async fn descendants(&self, tenant: &TenantId, root: ExecutionId) -> Result<Vec<Execution>> {
        let mut out: Vec<Execution> = Vec::new();
        let mut frontier = vec![root];
        while let Some(next) = frontier.pop() {
            for child in self.children(tenant, next).await? {
                // A guard against a tree that somehow loops: nothing is visited twice.
                if child.id != root && !out.iter().any(|e| e.id == child.id) {
                    frontier.insert(0, child.id);
                    out.push(child);
                }
            }
            if out.len() > 10_000 {
                break;
            }
        }
        Ok(out)
    }

    /// Executions of every tenant that are in one of these states (used by recovery and by the scheduler).
    async fn in_states(&self, states: &[ExecutionState]) -> Result<Vec<Execution>>;

    /// Events of an execution with `seq` greater than `after_seq`, in order.
    async fn events(&self, id: ExecutionId, after_seq: u64, limit: usize) -> Result<Vec<ExecutionEvent>>;

    /// The latest checkpoint.
    async fn checkpoint(&self, id: ExecutionId) -> Result<Option<Checkpoint>>;

    /// The tool call records, in the order the model asked.
    async fn tool_calls(&self, id: ExecutionId) -> Result<Vec<ToolCallRecord>>;

    /// The approvals of an execution, oldest first.
    async fn approvals(&self, id: ExecutionId) -> Result<Vec<Approval>>;

    /// One approval by id.
    async fn approval(&self, id: ApprovalId) -> Result<Option<Approval>>;

    /// Take (or extend, when `owner` already holds it) the right to work on an execution for `ttl_ms`.
    /// Returns false when someone else holds an unexpired lease.
    async fn acquire_lease(&self, id: ExecutionId, owner: &str, now: Timestamp, ttl_ms: u64) -> Result<bool>;

    /// Give the lease back.
    async fn release_lease(&self, id: ExecutionId, owner: &str) -> Result<()>;

    /// Drop every lease, whoever holds it. Only for a host that knows no other process uses this store (it holds the
    /// data folder's lock), at start-up, when the leases left over belong to processes that are gone.
    async fn release_all_leases(&self) -> Result<u64>;
}
