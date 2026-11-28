//! ZORIX core: the execution model, the lifecycle and the contracts every other part depends on.
//!
//! This crate has no I/O and no knowledge of providers, storage or interfaces. See `docs/architecture.md`.

pub mod access;
pub mod agent;
pub mod approval;
pub mod ask;
pub mod autonomy;
pub mod context;
pub mod delegation;
pub mod engine;
pub mod error;
pub mod estimate;
pub mod event;
pub mod execution;
pub mod ids;
pub mod integrations;
pub mod memory;
pub mod message;
pub mod model;
pub mod org;
pub mod permission;
pub mod policy;
pub mod ports;
pub mod project;
pub mod reason;
pub mod redact;
pub mod repository;
pub mod secret;
pub mod settings;
pub mod state;
pub mod time;
pub mod tokens;
pub mod tool;
pub mod untrusted;
pub mod workflow;

pub use agent::{
    Agent, AgentBinding, AgentConversation, AgentModels, AgentSlug, AgentSpec, AgentStore, AgentVersion, Avatar,
    CheckOutcome, EVAL_LABEL, EvalCase, EvalCheck, EvalJudge, EvalResult, EvalRun, JudgeOutcome, MAX_AGENTS, MAX_TEAMS,
    MAX_VERSIONS, SkillFile, SkillPin, SkillRecord, StoredCase, Team, model_allowed, valid_skill_name,
    validate_model_pattern,
};
pub use approval::{Approval, ApprovalDecision, ApprovalStatus, call_hash, canonical_hash};
pub use ask::{
    ASK_TIMEOUT, Answer, CallHost, ElicitReply, FieldKind, FormField, MAX_ASKS_PER_CALL, MAX_CHOICES, MAX_FIELDS,
    MAX_QUESTION_BYTES, MAX_SAMPLE_BYTES, MAX_SAMPLE_MESSAGES, MAX_SAMPLE_TOKENS, PendingAsk, Question, SampleMessage,
    SampleReply, SampleRequest, SampleRole, ServerAsk, time_left,
};
pub use autonomy::{Autonomy, AutonomyLevel};
pub use context::{ContextBuilder, ContextSegment, ContextSource, ExecutionView, Priority};
pub use delegation::{
    ChildResult, DelegateRequest, DelegateTask, DelegateTool, DelegationPlanner, MAX_DEPTH, MAX_DESCENDANTS, MAX_TASKS,
    PlanContext, PlannedChild, child_key,
};
pub use engine::{ExecutionDetail, ExecutionEngine, LiveEvent, RecoveryReport, TreeApproval};
pub use error::{Error, Result};
pub use estimate::{EstimateBasis, EstimateInput, MAX_SAMPLES, MIN_SAMPLES, RunEstimate, RunSample, estimate_run};
pub use event::{EVENT_SCHEMA_VERSION, EventKind, ExecutionEvent, NewEvent};
pub use execution::{
    Attachment, Execution, ExecutionResult, Failure, FailureKind, LimitKind, Limits, MAX_ATTACHMENT_CHARS,
    MAX_ATTACHMENTS, MAX_ATTACHMENTS_CHARS, NewExecution, TaskRequest, Usage,
};
pub use ids::{
    AgentId, ApprovalId, AskId, ConnectionId, ConversationId, EvalCaseId, EvalRunId, ExecutionId, IdError,
    KnowledgeRootId, McpServerId, MemoryId, OrgReportId, ProjectId, TaskId, TenantId, ToolCallId, WaitId, WorkflowId,
    WorkflowRunId,
};
pub use integrations::{
    Connection, ConnectionKind, ConnectionPatch, ConnectionTest, EnvValue, EnvVar, HttpAuth, IntegrationStore,
    McpServer, McpToolPin, McpTransport, NewConnection, NewMcpServer, ObservedTool, PinAction, PinStatus,
    SandboxFolder, SandboxMode, ServerSandbox, ToolTrust, valid_name,
};
pub use memory::{
    ChunkView, Corpus, DocState, DocStatus, DocumentWrite, KnowledgeRoot, MemoryFilter, MemoryItem, MemoryKind,
    MemoryOptions, MemoryOrigin, MemoryPatch, MemorySettings, MemoryStatus, MemoryStore, NewChunk, NewMemory, NewRoot,
    OCR_KIND, OperationRecord, RecallMode, RecallSnapshot, Remembered, SCAN_NOTE, SCAN_UNREAD_NOTE, ScanCounts, Scope,
    TextHit, VectorHit, Visibility, bytes_hash, chunk_key, memory_key, operation_key, text_hash,
};
pub use message::{Message, NativeTurn, Provenance, Role, ToolCallRequest};
pub use model::{
    Effort, EmbedPurpose, EmbeddingProfile, EmbeddingRequest, EmbeddingResponse, FinishReason, GenerationParams,
    ModelError, ModelErrorKind, ModelGateway, ModelProfile, ModelRequest, ModelResponse, NoSink, RouteAttempt,
    RouteTrace, SegmentReport, StepPhase, TextSink,
};
pub use org::{
    DEFAULT_TASK_TIME_MS, Department, MAX_DEPARTMENTS, MAX_WORKERS, OrgEvent, OrgEventDraft, OrgEventKind, OrgReport,
    OrgSettings, OrgStore, ReportKind, ReportStatus, Worker, WorkerSchedule, validate_org,
};
pub use permission::{
    Action, Grants, Permission, network_grant, tool_grant, validate_host_pattern, validate_tool_pattern,
    validate_workspace_pattern, workspace_grant,
};
pub use policy::{PolicyDecision, PolicyEngine, PolicyRequest};
pub use ports::{EventPublisher, ExecutionScheduler, ResultValidator, Validation};
pub use project::{
    Date, Deliverable, DeliverableKind, MAX_DELIVERABLES, MAX_DEPENDENCIES, MAX_LINKS, MAX_PROJECT_TASKS, MAX_PROJECTS,
    MAX_TASK_RUNS, NeedsYou, Project, ProjectEvent, ProjectEventDraft, ProjectEventKind, ProjectKey, ProjectStore,
    RunOutcome, RunRequest, Task, TaskCount, TaskKey, TaskLink, TaskPriority, TaskRun, TaskState, Who,
    check_dependencies,
};
pub use reason::{CODES, Coded, Reason};
pub use redact::{redact, redact_text};
pub use repository::{
    Checkpoint, Commit, Committed, CreateOutcome, ExecutionRepository, HistorySummary, ListFilter, LoopState,
    ToolCallRecord, ToolCallStatus,
};
pub use secret::{REDACTED, Secret};
pub use settings::{
    AppSettings, ContextSettings, KnowledgeSettings, MAX_RETENTION_DAYS, MIN_RETENTION_DAYS, RetentionMode,
    RetentionReport, RetentionSettings, SettingsCell, SettingsStore, TelemetrySettings,
};
pub use state::{ExecutionState, InvalidTransition, Trigger, transition};
pub use time::{Clock, ManualClock, SystemClock, Timestamp};
pub use tokens::{Calibration, InMemoryCalibration, NoCalibration, TokenCalibration, TokenSample};
pub use tool::{
    PinnedTool, Tool, ToolAnnotations, ToolContext, ToolDefinition, ToolError, ToolRegistry, ToolSource, ToolSpec,
};
pub use workflow::{
    Ask, Confirm, Confirmation, Day, Edge, EventOn, Fired, MAX_ACTIVE_STEPS, MAX_CHAIN, MAX_LOOP_ITEMS, MAX_STEPS,
    MAX_WORKFLOW_VERSIONS, MAX_WORKFLOWS, Remind, RunState, Service, Step, StepClass, StepKind, StepRun, StepState,
    Then, TriggerState, WaitDecision, WaitKind, WaitStatus, When, Workflow, WorkflowRun, WorkflowSlug, WorkflowSpec,
    WorkflowStore, WorkflowVersion, WorkflowWait, wait_fingerprint,
};
