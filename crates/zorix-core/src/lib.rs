//! ZORIX core: the execution model, the lifecycle and the contracts every other part depends on.
//!
//! This crate has no I/O and no knowledge of providers, storage or interfaces. See `docs/architecture.md`.

pub mod approval;
pub mod context;
pub mod engine;
pub mod error;
pub mod event;
pub mod execution;
pub mod ids;
pub mod message;
pub mod model;
pub mod permission;
pub mod policy;
pub mod ports;
pub mod repository;
pub mod state;
pub mod time;
pub mod tool;

pub use approval::{Approval, ApprovalDecision, ApprovalStatus, call_hash};
pub use context::{ContextBuilder, ContextSegment, ContextSource, ExecutionView, Priority};
pub use engine::{ExecutionDetail, ExecutionEngine, RecoveryReport};
pub use error::{Error, Result};
pub use event::{EVENT_SCHEMA_VERSION, EventKind, ExecutionEvent, NewEvent};
pub use execution::{
    Execution, ExecutionResult, Failure, FailureKind, LimitKind, Limits, NewExecution, TaskRequest, Usage,
};
pub use ids::{ApprovalId, ConversationId, ExecutionId, IdError, TenantId, ToolCallId};
pub use message::{Message, Provenance, Role, ToolCallRequest};
pub use model::{FinishReason, ModelError, ModelGateway, ModelRequest, ModelResponse};
pub use permission::{
    Action, Grants, Permission, tool_grant, validate_tool_pattern, validate_workspace_pattern, workspace_grant,
};
pub use policy::{PolicyDecision, PolicyEngine, PolicyRequest};
pub use ports::{EventPublisher, ExecutionScheduler, ResultValidator, Validation};
pub use repository::{
    Checkpoint, Commit, Committed, CreateOutcome, ExecutionRepository, ListFilter, LoopState, ToolCallRecord,
    ToolCallStatus,
};
pub use state::{ExecutionState, InvalidTransition, Trigger, transition};
pub use time::{Clock, ManualClock, SystemClock, Timestamp};
pub use tool::{Tool, ToolAnnotations, ToolContext, ToolDefinition, ToolError, ToolRegistry, ToolSpec};
