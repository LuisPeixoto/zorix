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
}
