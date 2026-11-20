//! ZORIX core: the execution model, the lifecycle and the contracts every other part depends on.
//!
//! This crate has no I/O and no knowledge of providers, storage or interfaces. See `docs/architecture.md`.

pub mod access;
pub mod agent;
pub mod approval;
pub mod context;
pub mod delegation;
pub mod engine;
pub mod error;
pub mod event;
pub mod execution;
pub mod ids;
pub mod integrations;
pub mod memory;
pub mod message;
pub mod model;
pub mod permission;
pub mod policy;
pub mod ports;
pub mod redact;
pub mod repository;
pub mod secret;
pub mod state;
pub mod time;
pub mod tokens;
pub mod tool;

pub use agent::{
    Agent, AgentBinding, AgentConversation, AgentModels, AgentSlug, AgentSpec, AgentStore, AgentVersion, Avatar,
    CheckOutcome, EVAL_LABEL, EvalCase, EvalCheck, EvalJudge, EvalResult, EvalRun, JudgeOutcome, MAX_AGENTS, MAX_TEAMS,
    MAX_VERSIONS, SkillFile, SkillPin, SkillRecord, StoredCase, Team, model_allowed, valid_skill_name,
    validate_model_pattern,
};
pub use approval::{Approval, ApprovalDecision, ApprovalStatus, call_hash, canonical_hash};
pub use context::{ContextBuilder, ContextSegment, ContextSource, ExecutionView, Priority};
pub use delegation::{
    ChildResult, DelegateRequest, DelegateTask, DelegationPlanner, MAX_DEPTH, MAX_DESCENDANTS, MAX_TASKS, PlanContext,
    PlannedChild, child_key,
};
pub use engine::{ExecutionDetail, ExecutionEngine, LiveEvent, RecoveryReport, TreeApproval};
pub use error::{Error, Result};
pub use event::{EVENT_SCHEMA_VERSION, EventKind, ExecutionEvent, NewEvent};
pub use execution::{
    Execution, ExecutionResult, Failure, FailureKind, LimitKind, Limits, NewExecution, TaskRequest, Usage,
};
pub use ids::{
    AgentId, ApprovalId, ConnectionId, ConversationId, EvalCaseId, EvalRunId, ExecutionId, IdError, KnowledgeRootId,
    McpServerId, MemoryId, TenantId, ToolCallId,
};
pub use integrations::{
    Connection, ConnectionKind, ConnectionPatch, ConnectionTest, EnvValue, EnvVar, HttpAuth, IntegrationStore,
    McpServer, McpToolPin, McpTransport, NewConnection, NewMcpServer, ObservedTool, PinAction, PinStatus, ToolTrust,
    valid_name,
};
pub use memory::{
    ChunkView, Corpus, DocState, DocStatus, DocumentWrite, KnowledgeRoot, MemoryFilter, MemoryItem, MemoryKind,
    MemoryOptions, MemoryOrigin, MemoryPatch, MemorySettings, MemoryStatus, MemoryStore, NewChunk, NewMemory, NewRoot,
    OperationRecord, RecallMode, RecallSnapshot, Remembered, Scope, TextHit, VectorHit, Visibility, bytes_hash,
    chunk_key, memory_key, operation_key, text_hash,
};
pub use message::{Message, NativeTurn, Provenance, Role, ToolCallRequest};
pub use model::{
    Effort, EmbedPurpose, EmbeddingProfile, EmbeddingRequest, EmbeddingResponse, FinishReason, GenerationParams,
    ModelError, ModelErrorKind, ModelGateway, ModelProfile, ModelRequest, ModelResponse, NoSink, RouteAttempt,
    RouteTrace, SegmentReport, StepPhase, TextSink,
};
pub use permission::{
    Action, Grants, Permission, network_grant, tool_grant, validate_host_pattern, validate_tool_pattern,
    validate_workspace_pattern, workspace_grant,
};
pub use policy::{PolicyDecision, PolicyEngine, PolicyRequest};
pub use ports::{EventPublisher, ExecutionScheduler, ResultValidator, Validation};
pub use redact::{redact, redact_text};
pub use repository::{
    Checkpoint, Commit, Committed, CreateOutcome, ExecutionRepository, ListFilter, LoopState, ToolCallRecord,
    ToolCallStatus,
};
pub use secret::{REDACTED, Secret};
pub use state::{ExecutionState, InvalidTransition, Trigger, transition};
pub use time::{Clock, ManualClock, SystemClock, Timestamp};
pub use tokens::{Calibration, InMemoryCalibration, NoCalibration, TokenCalibration, TokenSample};
pub use tool::{
    PinnedTool, Tool, ToolAnnotations, ToolContext, ToolDefinition, ToolError, ToolRegistry, ToolSource, ToolSpec,
};
