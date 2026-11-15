//! ZORIX core: the execution model, the lifecycle and the contracts every other part depends on.
//!
//! This crate has no I/O and no knowledge of providers, storage or interfaces. See `docs/architecture.md`.

pub mod error;
pub mod execution;
pub mod ids;
pub mod permission;
pub mod state;
pub mod time;

pub use error::{Error, Result};
pub use execution::{
    Execution, ExecutionResult, Failure, FailureKind, LimitKind, Limits, NewExecution, TaskRequest, Usage,
};
pub use ids::{ApprovalId, ConversationId, ExecutionId, IdError, TenantId, ToolCallId};
pub use permission::{Action, Grants, Permission};
pub use state::{ExecutionState, InvalidTransition, Trigger, transition};
pub use time::{Clock, ManualClock, SystemClock, Timestamp};
