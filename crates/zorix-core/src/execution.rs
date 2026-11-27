//! The execution record: what was asked, under which limits and grants, and where it stands.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::AgentBinding;
use crate::error::{Error, Result};
use crate::ids::{ConversationId, ExecutionId, TenantId};
use crate::memory::MemoryOptions;
use crate::model::GenerationParams;
use crate::permission::Grants;
use crate::state::{ExecutionState, InvalidTransition, Trigger, transition};
use crate::time::Timestamp;

/// The activity to carry out.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskRequest {
    /// Standing instructions for the model (the "system" text). Optional.
    #[serde(default)]
    pub instructions: Option<String>,
    /// What to do.
    pub prompt: String,
    /// Structured data that goes with the request. The model sees it as data.
    #[serde(default)]
    pub input: Option<Value>,
    /// Which model to use. The runtime does not interpret it: the `ModelGateway` does.
    pub model: String,
    /// A JSON Schema the final answer must satisfy, when the answer is structured.
    #[serde(default)]
    pub expected_output: Option<Value>,
    /// Free labels for the client (a task id, a screen). Never sent to the model.
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    /// Output size and effort for the model calls of this execution.
    #[serde(default)]
    pub params: GenerationParams,
}

/// What can go wrong with an execution, as numbers. Every limit is checked on every iteration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Limits {
    /// Most model calls that lead to another step.
    pub max_iterations: u32,
    /// Most tool calls (requested, whether they ran or not).
    pub max_tool_calls: u32,
    /// Most tokens, input plus output, over the whole execution.
    pub max_total_tokens: Option<u64>,
    /// Most cost, in millionths of a dollar.
    pub max_cost_micros: Option<u64>,
    /// Most time of work, in milliseconds. Time spent waiting for a person does not count.
    pub max_active_ms: u64,
    /// How long one model call may take, in milliseconds.
    pub model_timeout_ms: u64,
    /// Attempts per model call when the error is temporary.
    pub max_model_attempts: u32,
    /// How many times a rejected final answer is sent back to the model.
    pub max_validation_retries: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_iterations: 12,
            max_tool_calls: 24,
            max_total_tokens: None,
            max_cost_micros: None,
            max_active_ms: 300_000,
            model_timeout_ms: 60_000,
            max_model_attempts: 3,
            max_validation_retries: 1,
        }
    }
}

impl Limits {
    fn validate(&self) -> Result<()> {
        let bad = |what: &str| Err(Error::InvalidInput(what.to_string()));
        if !(1..=1_000).contains(&self.max_iterations) {
            return bad("max_iterations must be between 1 and 1000.");
        }
        if self.max_tool_calls > 10_000 {
            return bad("max_tool_calls must be 10000 or less.");
        }
        if self.max_active_ms == 0 || self.model_timeout_ms == 0 {
            return bad("Time limits must be greater than zero.");
        }
        if !(1..=10).contains(&self.max_model_attempts) {
            return bad("max_model_attempts must be between 1 and 10.");
        }
        if self.max_validation_retries > 10 {
            return bad("max_validation_retries must be 10 or less.");
        }
        Ok(())
    }
}

/// Which limit was reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitKind {
    /// `max_iterations`.
    Iterations,
    /// `max_tool_calls`.
    ToolCalls,
    /// `max_total_tokens`.
    Tokens,
    /// `max_cost_micros`.
    Cost,
    /// `max_active_ms`.
    ActiveTime,
}

/// What an execution has consumed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Usage {
    /// Tokens sent to models that were neither read from nor written to a provider's cache. Adapters normalise each
    /// provider's numbers into this shape, so cost is computed the same way for all of them.
    pub input_tokens: u64,
    /// Tokens produced by models.
    pub output_tokens: u64,
    /// Cost in millionths of a dollar. Zero when no price is known.
    pub cost_micros: u64,
    /// Input tokens served from the provider's prompt cache.
    pub cache_read_tokens: u64,
    /// Input tokens written to the provider's prompt cache.
    pub cache_write_tokens: u64,
    /// Tokens the model spent thinking (when the provider reports them separately).
    pub reasoning_tokens: u64,
    /// Model calls that returned.
    pub model_calls: u32,
    /// Tool calls that ran.
    pub tool_calls: u32,
}

impl Usage {
    /// Input plus output tokens.
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }

    /// Add another amount.
    pub fn add(&mut self, other: &Usage) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.cost_micros = self.cost_micros.saturating_add(other.cost_micros);
        self.cache_read_tokens = self.cache_read_tokens.saturating_add(other.cache_read_tokens);
        self.cache_write_tokens = self.cache_write_tokens.saturating_add(other.cache_write_tokens);
        self.reasoning_tokens = self.reasoning_tokens.saturating_add(other.reasoning_tokens);
        self.model_calls = self.model_calls.saturating_add(other.model_calls);
        self.tool_calls = self.tool_calls.saturating_add(other.tool_calls);
    }
}

/// Why an execution failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// A limit was reached; see [`Failure::limit`].
    LimitExceeded,
    /// The model failed and retrying did not help.
    Model,
    /// A tool failed in a way the model could not work around.
    Tool,
    /// The final answer was rejected too many times.
    Validation,
    /// A person refused something that the activity could not go on without.
    Rejected,
    /// The runtime could not continue.
    Internal,
}

/// The reason an execution ended in `Failed`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    /// The category.
    pub kind: FailureKind,
    /// A sentence for a person.
    pub message: String,
    /// Which limit, when `kind` is `LimitExceeded`.
    #[serde(default)]
    pub limit: Option<LimitKind>,
}

impl Failure {
    /// A failure of some kind.
    pub fn new(kind: FailureKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into(), limit: None }
    }

    /// A limit was reached.
    pub fn limit(limit: LimitKind, message: impl Into<String>) -> Self {
        Self { kind: FailureKind::LimitExceeded, message: message.into(), limit: Some(limit) }
    }
}

/// The accepted final answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExecutionResult {
    /// The answer as text.
    pub text: String,
    /// The answer as data, when the request expected structured output.
    #[serde(default)]
    pub structured: Option<Value>,
}

/// What a client sends to start an activity.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NewExecution {
    /// The owner. Defaults to the local tenant.
    #[serde(default)]
    pub tenant: Option<TenantId>,
    /// The conversation it belongs to, if any.
    #[serde(default)]
    pub conversation: Option<ConversationId>,
    /// The execution that asked for this one, when work was delegated (ADR-0029).
    #[serde(default)]
    pub parent: Option<ExecutionId>,
    /// The execution at the top of the tree this one belongs to. `None` for a root.
    #[serde(default)]
    pub root: Option<ExecutionId>,
    /// How deep it is: 0 for a root, 1 for its children.
    #[serde(default)]
    pub depth: u32,
    /// The activity.
    pub request: TaskRequest,
    /// The limits.
    #[serde(default)]
    pub limits: Limits,
    /// What it may do. Nothing by default.
    #[serde(default)]
    pub grants: Grants,
    /// The folder the file tools are confined to (an absolute path). Without one, file tools refuse every call.
    #[serde(default)]
    pub workspace: Option<PathBuf>,
    /// Sending the same key again returns the same execution instead of creating a second one.
    #[serde(default)]
    pub idempotency_key: Option<String>,
    /// What the execution may use from memory. Everything, by default.
    #[serde(default)]
    pub memory: MemoryOptions,
    /// The agent version it comes from, when an agent made it (ADR-0024). Frozen at creation.
    #[serde(default)]
    pub agent: Option<AgentBinding>,
}

impl NewExecution {
    /// Most characters in a prompt, instructions text or key.
    pub const MAX_TEXT: usize = 200_000;

    /// Check the request before anything is stored.
    pub fn validate(&self) -> Result<()> {
        let bad = |what: &str| Err(Error::InvalidInput(what.to_string()));
        if self.request.prompt.trim().is_empty() {
            return bad("The prompt is empty.");
        }
        if self.request.prompt.chars().count() > Self::MAX_TEXT {
            return bad("The prompt is too long (200000 characters at most).");
        }
        if self.request.instructions.as_ref().is_some_and(|text| text.chars().count() > Self::MAX_TEXT) {
            return bad("The instructions are too long (200000 characters at most).");
        }
        if self.request.model.trim().is_empty() {
            return bad("Choose a model.");
        }
        if let Some(key) = &self.idempotency_key
            && (key.trim().is_empty() || key.len() > 128)
        {
            return bad("The idempotency key must have 1 to 128 characters.");
        }
        if let Some(workspace) = &self.workspace
            && !workspace.is_absolute()
        {
            return bad("The workspace must be an absolute path.");
        }
        if let Some(agent) = &self.agent
            && agent.allowed_models.is_empty()
        {
            return bad("An agent's run must name the models it may use (`*` means any).");
        }
        self.limits.validate()
    }
}

/// One execution, as stored.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Execution {
    /// Its identity.
    pub id: ExecutionId,
    /// The owner.
    pub tenant: TenantId,
    /// The conversation it came from, if any.
    pub conversation: Option<ConversationId>,
    /// The execution that asked for it, if any.
    pub parent: Option<ExecutionId>,
    /// The execution at the top of its tree. `None` for a root.
    #[serde(default)]
    pub root: Option<ExecutionId>,
    /// How deep it is: 0 for a root.
    #[serde(default)]
    pub depth: u32,
    /// The activity.
    pub request: TaskRequest,
    /// Where it stands.
    pub state: ExecutionState,
    /// Incremented by the repository on every committed change. Used to detect concurrent changes.
    pub version: u64,
    /// The limits.
    pub limits: Limits,
    /// What it may do.
    pub grants: Grants,
    /// The folder its file tools are confined to.
    #[serde(default)]
    pub workspace: Option<PathBuf>,
    /// The idempotency key it was created with.
    pub idempotency_key: Option<String>,
    /// What it may use from memory. Executions stored by 0.2 have everything on.
    #[serde(default)]
    pub memory: MemoryOptions,
    /// The agent version it comes from, when an agent made it (ADR-0024). Executions stored before 0.5 have none.
    #[serde(default)]
    pub agent: Option<AgentBinding>,
    /// The tools this execution may use, pinned when its first model step starts (ADR-0020). `None` until then, and for
    /// executions stored by 0.3, which pin at their next step.
    #[serde(default)]
    pub tool_pins: Option<Vec<crate::tool::PinnedTool>>,
    /// How many model-driven iterations have started.
    pub iteration: u32,
    /// How many tool calls have been requested.
    pub tool_calls_requested: u32,
    /// How many times a final answer was rejected so far.
    pub validation_retries: u32,
    /// Milliseconds of work done so far (waiting for a person is not counted).
    pub active_ms: u64,
    /// What has been consumed.
    pub usage: Usage,
    /// The accepted answer, once completed.
    pub result: Option<ExecutionResult>,
    /// Why it failed, once failed.
    pub failure: Option<Failure>,
    /// Why it was cancelled, once cancelled.
    pub cancel_reason: Option<String>,
    /// When it was created.
    pub created_at: Timestamp,
    /// When it last changed.
    pub updated_at: Timestamp,
    /// When a worker first started it.
    pub started_at: Option<Timestamp>,
    /// When it reached a terminal state.
    pub finished_at: Option<Timestamp>,
    /// Retention dropped its events, steps, approvals and recall (it keeps the request, the answer, the usage and the time). Never set by a run.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub compacted: bool,
}

impl Execution {
    /// A new execution in the `Created` state.
    pub fn new(id: ExecutionId, new: NewExecution, now: Timestamp) -> Self {
        Self {
            id,
            tenant: new.tenant.unwrap_or_else(TenantId::local),
            conversation: new.conversation,
            parent: new.parent,
            root: new.root,
            depth: new.depth,
            request: new.request,
            state: ExecutionState::Created,
            version: 0,
            limits: new.limits,
            grants: new.grants,
            workspace: new.workspace,
            idempotency_key: new.idempotency_key,
            memory: new.memory,
            agent: new.agent,
            tool_pins: None,
            iteration: 0,
            tool_calls_requested: 0,
            validation_retries: 0,
            active_ms: 0,
            usage: Usage::default(),
            result: None,
            failure: None,
            cancel_reason: None,
            created_at: now,
            updated_at: now,
            started_at: None,
            finished_at: None,
            compacted: false,
        }
    }

    /// Move to the next state, if the lifecycle allows it, and keep the timestamps.
    pub fn apply(&mut self, trigger: Trigger, now: Timestamp) -> std::result::Result<(), InvalidTransition> {
        let next = transition(self.state, trigger)?;
        self.state = next;
        self.updated_at = now;
        if next == ExecutionState::Running && self.started_at.is_none() {
            self.started_at = Some(now);
        }
        if next.is_terminal() {
            self.finished_at = Some(now);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> NewExecution {
        NewExecution {
            request: TaskRequest { prompt: "hello".into(), model: "sim".into(), ..TaskRequest::default() },
            ..NewExecution::default()
        }
    }

    #[test]
    fn a_new_execution_is_created_with_the_local_tenant_and_no_grants() {
        let execution = Execution::new(ExecutionId::new(), request(), Timestamp(10));
        assert_eq!(execution.state, ExecutionState::Created);
        assert_eq!(execution.tenant, TenantId::local());
        assert_eq!(execution.version, 0);
        assert_eq!(execution.grants, Grants::none());
        assert_eq!(execution.created_at, Timestamp(10));
    }

    #[test]
    fn apply_keeps_the_timestamps() {
        let mut execution = Execution::new(ExecutionId::new(), request(), Timestamp(10));
        execution.apply(Trigger::Enqueue, Timestamp(20)).unwrap();
        execution.apply(Trigger::Start, Timestamp(30)).unwrap();
        assert_eq!(execution.started_at, Some(Timestamp(30)));
        execution.apply(Trigger::AwaitModel, Timestamp(40)).unwrap();
        execution.apply(Trigger::StepFinished, Timestamp(50)).unwrap();
        execution.apply(Trigger::Complete, Timestamp(60)).unwrap();
        assert_eq!(execution.finished_at, Some(Timestamp(60)));
        assert_eq!(execution.started_at, Some(Timestamp(30)), "the first start is kept");
        assert_eq!(execution.updated_at, Timestamp(60));
    }

    #[test]
    fn apply_rejects_a_step_the_lifecycle_does_not_allow_and_changes_nothing() {
        let mut execution = Execution::new(ExecutionId::new(), request(), Timestamp(10));
        let before = execution.clone();
        assert!(execution.apply(Trigger::Complete, Timestamp(20)).is_err());
        assert_eq!(execution, before);
    }

    #[test]
    fn validation_rejects_bad_requests() {
        assert!(request().validate().is_ok());

        let mut empty = request();
        empty.request.prompt = "   ".into();
        assert!(empty.validate().is_err());

        let mut no_model = request();
        no_model.request.model = String::new();
        assert!(no_model.validate().is_err());

        let mut long = request();
        long.request.prompt = "x".repeat(NewExecution::MAX_TEXT + 1);
        assert!(long.validate().is_err());

        let mut key = request();
        key.idempotency_key = Some(String::new());
        assert!(key.validate().is_err());

        let mut limits = request();
        limits.limits.max_iterations = 0;
        assert!(limits.validate().is_err());

        let mut relative = request();
        relative.workspace = Some(PathBuf::from("docs"));
        assert!(relative.validate().is_err(), "a workspace must be absolute");
        let mut absolute = request();
        absolute.workspace = Some(std::env::temp_dir());
        assert!(absolute.validate().is_ok());
    }

    #[test]
    fn usage_adds_without_overflowing() {
        let mut total = Usage { input_tokens: u64::MAX, ..Usage::default() };
        total.add(&Usage {
            input_tokens: 5,
            output_tokens: 7,
            cost_micros: 3,
            model_calls: 1,
            tool_calls: 2,
            ..Usage::default()
        });
        assert_eq!(total.input_tokens, u64::MAX);
        assert_eq!(total.output_tokens, 7);
        assert_eq!(total.total_tokens(), u64::MAX);
    }

    #[test]
    fn usage_from_version_0_1_loads_with_the_new_counters_at_zero() {
        let usage: Usage = serde_json::from_str(
            r#"{"input_tokens":4,"output_tokens":2,"cost_micros":6,"model_calls":1,"tool_calls":0}"#,
        )
        .unwrap();
        assert_eq!((usage.input_tokens, usage.cache_read_tokens, usage.reasoning_tokens), (4, 0, 0));
    }

    #[test]
    fn limits_have_sensible_defaults_and_accept_partial_json() {
        let limits: Limits = serde_json::from_str(r#"{"max_iterations": 3}"#).unwrap();
        assert_eq!(limits.max_iterations, 3);
        assert_eq!(limits.max_tool_calls, Limits::default().max_tool_calls);
    }
}

#[cfg(test)]
mod memory_options_tests {
    use super::*;

    #[test]
    fn an_execution_stored_before_memory_existed_still_loads_with_everything_on() {
        let new = NewExecution {
            request: TaskRequest { prompt: "hi".into(), ..TaskRequest::default() },
            ..NewExecution::default()
        };
        let execution = Execution::new(ExecutionId::new(), new, Timestamp(1));
        let mut json = serde_json::to_value(&execution).unwrap();
        json.as_object_mut().unwrap().remove("memory").expect("the field is written");
        let loaded: Execution = serde_json::from_value(json).unwrap();
        assert_eq!(loaded.memory, MemoryOptions::default());
        // and a request without the field is read the same way
        let request: NewExecution = serde_json::from_str(r#"{"request":{"prompt":"hi","model":"demo"}}"#).unwrap();
        assert_eq!(request.memory, MemoryOptions::default());
    }

    #[test]
    fn what_a_run_chose_is_kept() {
        let new = NewExecution {
            request: TaskRequest { prompt: "hi".into(), ..TaskRequest::default() },
            memory: MemoryOptions::none(),
            ..NewExecution::default()
        };
        assert_eq!(Execution::new(ExecutionId::new(), new, Timestamp(1)).memory, MemoryOptions::none());
    }
}
