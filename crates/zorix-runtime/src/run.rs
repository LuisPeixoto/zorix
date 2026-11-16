//! The loop that drives one execution, step by step, writing a checkpoint as it goes.
//!
//! Every change of state is one atomic commit (state + events + checkpoint + records). If this process dies between
//! two commits, `Engine::recover` continues from the last one.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use tracing::Instrument;
use zorix_core::{
    Approval, ApprovalId, ApprovalStatus, Checkpoint, Commit, Error, EventKind, Execution, ExecutionId,
    ExecutionResult, ExecutionState, ExecutionView, Failure, FailureKind, LoopState, Message, ModelError, ModelRequest,
    ModelResponse, NewEvent, Permission, PolicyDecision, PolicyRequest, Role, StepPhase, Timestamp, Tool, ToolCallId,
    ToolCallRecord, ToolCallRequest, ToolCallStatus, ToolContext, ToolSpec, Trigger, Validation, call_hash,
};

use crate::engine::{Engine, RunControl};
use crate::tools::{CallError, ToolExecutor};
use crate::validators::parse_structured;

/// Why a run ended without finishing the loop.
pub(crate) enum Stop {
    /// The execution reached a state where this worker has nothing more to do (finished, failed, cancelled, paused or
    /// waiting for a person). Everything is already written.
    Done,
    /// The engine is shutting down. The execution is left as it is.
    Shutdown,
    /// Somebody else changed the execution; this worker steps aside.
    Abandoned,
    /// Something broke that this worker cannot repair (for example the store is unreachable).
    Halt(String),
}

impl From<Error> for Stop {
    fn from(error: Error) -> Self {
        match error {
            Error::Conflict(_) => Stop::Abandoned,
            other => Stop::Halt(other.to_string()),
        }
    }
}

type Step<T = ()> = std::result::Result<T, Stop>;

/// Room left for what is not conversation: the provider's own framing and the tool definitions.
const CONTEXT_MARGIN_TOKENS: u64 = 1_024;

/// The smallest context budget, so a tiny reported window cannot starve the required parts.
const MIN_CONTEXT_TOKENS: u64 = 1_024;

/// `events` followed by `last`.
fn with(mut events: Vec<NewEvent>, last: NewEvent) -> Vec<NewEvent> {
    events.push(last);
    events
}

/// Records that travel with a commit.
#[derive(Default)]
struct Extras {
    tool_calls: Vec<ToolCallRecord>,
    approvals: Vec<Approval>,
    checkpoint: bool,
}

impl Extras {
    fn checkpoint() -> Self {
        Self { checkpoint: true, ..Self::default() }
    }
}

pub(crate) struct Run {
    engine: Engine,
    exec: Execution,
    state: LoopState,
    step: u64,
    control: Arc<RunControl>,
    tick: Timestamp,
    records: Vec<ToolCallRecord>,
    approvals: Vec<Approval>,
}

impl Engine {
    /// Work on one queued execution until it finishes, fails, is cancelled, pauses or needs a person.
    pub(crate) async fn drive(&self, id: ExecutionId) {
        let inner = &self.inner;
        match inner.repository.acquire_lease(id, &inner.owner, self.now(), inner.config.lease_ttl_ms).await {
            Ok(true) => {}
            Ok(false) => return self.try_again_later(id),
            Err(error) => return tracing::error!(execution = %id, %error, "could not take the lease"),
        }
        let control = Arc::new(RunControl {
            cancel: inner.shutdown.child_token(),
            reason: parking_lot::Mutex::new(None),
            pause: std::sync::atomic::AtomicBool::new(false),
        });
        inner.runs.lock().insert(id, control.clone());
        let heartbeat = self.heartbeat(id);

        // Span names and attributes follow the OpenTelemetry GenAI conventions, so an OTLP exporter can be added later
        // without changing the runtime. Nothing a person or a model wrote (prompts, arguments, results) goes in them.
        let span =
            tracing::info_span!("invoke_agent", gen_ai.operation.name = "invoke_agent", zorix.execution.id = %id);
        let drive = async {
            match Run::load(self.clone(), id, control).await {
                Ok(Some(mut run)) => match run.run().await {
                    Ok(()) | Err(Stop::Done) => {}
                    Err(Stop::Shutdown) => tracing::debug!("stopped for shutdown"),
                    Err(Stop::Abandoned) => tracing::warn!("changed by someone else; stepping aside"),
                    Err(Stop::Halt(why)) => tracing::error!(%why, "the run could not go on"),
                },
                Ok(None) => {}
                Err(error) => tracing::error!(%error, "could not load the execution"),
            }
        };
        drive.instrument(span).await;

        heartbeat.cancel();
        inner.runs.lock().remove(&id);
        if let Err(error) = inner.repository.release_lease(id, &inner.owner).await {
            tracing::warn!(execution = %id, %error, "could not release the lease");
        }
    }

    /// Somebody else holds the lease (a worker that is still finishing, or another process). Ask again shortly.
    fn try_again_later(&self, id: ExecutionId) {
        let engine = self.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(300)) => {}
                _ = engine.inner.shutdown.cancelled() => return,
            }
            let _ = engine.inner.scheduler.enqueue(id).await;
        });
    }

    /// Keep the lease alive while a long model or tool call runs.
    fn heartbeat(&self, id: ExecutionId) -> CancellationToken {
        let stop = CancellationToken::new();
        let (engine, token) = (self.clone(), stop.clone());
        tokio::spawn(async move {
            let every = Duration::from_millis((engine.inner.config.lease_ttl_ms / 3).max(50));
            loop {
                tokio::select! {
                    _ = token.cancelled() => return,
                    // A stopped (or crashed) engine must stop renewing, or its leases would never expire.
                    _ = engine.inner.shutdown.cancelled() => return,
                    _ = tokio::time::sleep(every) => {}
                }
                let renewed = engine
                    .inner
                    .repository
                    .acquire_lease(id, &engine.inner.owner, engine.now(), engine.inner.config.lease_ttl_ms)
                    .await;
                if !matches!(renewed, Ok(true)) {
                    tracing::warn!(execution = %id, "could not renew the lease");
                }
            }
        });
        stop
    }
}

impl Run {
    async fn load(engine: Engine, id: ExecutionId, control: Arc<RunControl>) -> zorix_core::Result<Option<Run>> {
        let repository = engine.inner.repository.clone();
        let Some(exec) = repository.load(id).await? else { return Ok(None) };
        if exec.state != ExecutionState::Queued {
            // A stale queue entry: the execution was cancelled, paused or already taken.
            return Ok(None);
        }
        let checkpoint = repository.checkpoint(id).await?;
        let (state, step) = checkpoint.map(|c| (c.state, c.step)).unwrap_or_default();
        let records = repository.tool_calls(id).await?;
        let approvals = repository.approvals(id).await?;
        let tick = engine.now();
        Ok(Some(Run { engine, exec, state, step, control, tick, records, approvals }))
    }

    fn event(&self, kind: EventKind, payload: Value) -> NewEvent {
        self.engine.event(kind, payload)
    }

    /// Apply the triggers, write everything in one atomic commit, publish the events.
    async fn commit(&mut self, triggers: &[Trigger], events: Vec<NewEvent>, extras: Extras) -> Step {
        let now = self.engine.now();
        for trigger in triggers {
            self.exec.apply(*trigger, now).map_err(|e| Stop::Halt(format!("internal: {e}")))?;
        }
        let mut commit = Commit::new(self.exec.clone(), self.exec.version);
        commit.events = events;
        commit.tool_calls = extras.tool_calls;
        commit.approvals = extras.approvals;
        if extras.checkpoint {
            self.step += 1;
            commit.checkpoint = Some(Checkpoint {
                execution_id: self.exec.id,
                step: self.step,
                state: self.state.clone(),
                created_at: now,
            });
        }
        let done = self.engine.commit(commit).await?;
        self.exec = done.execution;
        tracing::debug!(state = %self.exec.state, version = self.exec.version, "committed");
        Ok(())
    }

    fn upsert_record(&mut self, record: ToolCallRecord) {
        match self.records.iter_mut().find(|r| r.call_id == record.call_id) {
            Some(existing) => *existing = record,
            None => self.records.push(record),
        }
    }

    fn tick_time(&mut self) {
        let now = self.engine.now();
        self.exec.active_ms = self.exec.active_ms.saturating_add(now.since(self.tick));
        self.tick = now;
    }

    // ---- the loop ---------------------------------------------------------------------------------------------

    async fn run(&mut self) -> Step {
        let first = self.exec.started_at.is_none();
        let started =
            if first { self.event(EventKind::Started, json!({})) } else { self.event(EventKind::Resumed, json!({})) };
        self.commit(&[Trigger::Start], vec![started], Extras::default()).await?;
        loop {
            self.boundary().await?;
            if let Some(call) = self.state.pending.first().cloned() {
                self.tool_call(call).await?;
            } else {
                self.model_turn().await?;
            }
        }
    }

    /// Checks made between steps: shutdown, cancellation, pause and the time limit.
    async fn boundary(&mut self) -> Step {
        if self.engine.inner.shutdown.is_cancelled() {
            return Err(Stop::Shutdown);
        }
        if self.control.cancel.is_cancelled() {
            return Err(self.finish_cancelled().await);
        }
        if self.control.pause.load(Ordering::SeqCst) {
            let events = vec![self.event(EventKind::Paused, json!({ "step": self.step }))];
            self.commit(&[Trigger::Pause], events, Extras::default()).await?;
            return Err(Stop::Done);
        }
        self.tick_time();
        if let Some(failure) = self.engine.inner.policy.check_limits(&self.exec) {
            return Err(self.fail(failure).await);
        }
        Ok(())
    }

    async fn fail(&mut self, failure: Failure) -> Stop {
        self.exec.failure = Some(failure.clone());
        let event = self.event(
            EventKind::Failed,
            json!({ "kind": failure.kind, "message": failure.message, "limit": failure.limit, "usage": self.exec.usage }),
        );
        match self.commit(&[Trigger::Fail], vec![event], Extras::default()).await {
            Ok(()) => Stop::Done,
            Err(stop) => stop,
        }
    }

    async fn finish_cancelled(&mut self) -> Stop {
        if self.engine.inner.shutdown.is_cancelled() {
            return Stop::Shutdown;
        }
        let reason = self.control.reason.lock().clone();
        self.exec.cancel_reason = reason.clone();
        let now = self.engine.now();
        let denied: Vec<Approval> = self
            .approvals
            .iter()
            .filter(|a| a.status == ApprovalStatus::Pending)
            .cloned()
            .map(|mut a| {
                a.status = ApprovalStatus::Denied;
                a.decided_at = Some(now);
                a.decided_by = Some("system".into());
                a.note = Some("The execution was cancelled.".into());
                a
            })
            .collect();
        let event = self.event(EventKind::Cancelled, json!({ "reason": reason }));
        match self.commit(&[Trigger::Cancel], vec![event], Extras { approvals: denied, ..Extras::default() }).await {
            Ok(()) => Stop::Done,
            Err(stop) => stop,
        }
    }

    async fn cancelled_or_shutdown(&mut self) -> Stop {
        self.finish_cancelled().await
    }

    // ---- model ------------------------------------------------------------------------------------------------

    fn offered_tools(&self) -> Vec<ToolSpec> {
        self.engine
            .inner
            .tools
            .list()
            .into_iter()
            .filter(|spec| self.exec.grants.allows(&Permission::use_tool(&spec.id)))
            .collect()
    }

    async fn model_turn(&mut self) -> Step {
        self.exec.iteration += 1;
        if let Some(failure) = self.engine.inner.policy.check_limits(&self.exec) {
            return Err(self.fail(failure).await);
        }
        let tools = self.offered_tools();
        let context_budget = self.engine.inner.model.describe(&self.exec.request.model).map(|profile| {
            profile
                .context_window
                .saturating_sub(profile.max_output)
                .saturating_sub(CONTEXT_MARGIN_TOKENS)
                .max(MIN_CONTEXT_TOKENS)
        });
        let built = {
            let view = ExecutionView {
                execution: &self.exec,
                transcript: &self.state.transcript,
                tools: &tools,
                context_budget,
            };
            self.engine.inner.context.build(&view).await
        };
        let request = match built {
            Ok(mut request) => {
                request.phase = self.phase();
                request
            }
            Err(error) => {
                return Err(self
                    .fail(Failure::new(FailureKind::Internal, format!("Could not build the context: {error}")))
                    .await);
            }
        };
        let requested = self.event(
            EventKind::ModelRequested,
            json!({
                "iteration": self.exec.iteration,
                "model": request.model,
                "messages": request.messages.len(),
                "tools": request.tools.len(),
                "estimated_input_tokens": request.messages.iter().map(Message::estimated_tokens).sum::<u64>(),
            }),
        );
        self.commit(&[Trigger::AwaitModel], vec![requested], Extras::default()).await?;

        let response = self.call_model(request).await?;
        let mut usage = response.usage;
        usage.model_calls = usage.model_calls.max(1);
        usage.tool_calls = 0;
        self.exec.usage.add(&usage);

        let mut events = self.fallback_events(&response);
        let responded = self.event(
            EventKind::ModelResponded,
            json!({
                "iteration": self.exec.iteration,
                "model": response.model,
                "provider": response.provider,
                "route": response.route,
                "finish": response.finish,
                "text_chars": response.text.chars().count(),
                "tool_calls": response.tool_calls.len(),
                "usage": usage,
            }),
        );
        events.push(responded);
        if response.tool_calls.is_empty() {
            return self.final_answer(response, events).await;
        }

        let iteration = self.exec.iteration;
        let calls: Vec<ToolCallRequest> = response
            .tool_calls
            .into_iter()
            .enumerate()
            .map(|(index, mut call)| {
                call.id = ToolCallId(format!("call_{iteration}_{index}"));
                call
            })
            .collect();
        self.state.transcript.push(Message::assistant(response.text, calls.clone()).with_native(response.native));
        self.state.pending = calls;
        let progress = self.event(
            EventKind::Progress,
            json!({ "iteration": iteration, "tool_calls_requested": self.exec.tool_calls_requested, "active_ms": self.exec.active_ms }),
        );
        events.push(progress);
        self.commit(&[Trigger::StepFinished], events, Extras::checkpoint()).await
    }

    /// Where in the execution a model call is: the first, a continuation after tool results, or a retry after a rejection.
    fn phase(&self) -> StepPhase {
        match self.state.transcript.last() {
            None => StepPhase::Start,
            Some(message) if message.role == Role::User => StepPhase::Retry,
            Some(_) => StepPhase::Continue,
        }
    }

    /// One event for each candidate a router passed over before the one that answered.
    fn fallback_events(&self, response: &ModelResponse) -> Vec<NewEvent> {
        let Some(route) = &response.route else { return Vec::new() };
        route
            .attempts
            .windows(2)
            .map(|pair| {
                self.event(
                    EventKind::ModelFallback,
                    json!({
                        "from": format!("{}/{}", pair[0].provider, pair[0].model),
                        "to": format!("{}/{}", pair[1].provider, pair[1].model),
                        "reason": pair[0].outcome,
                    }),
                )
            })
            .collect()
    }

    /// One model call with a timeout and retries for temporary errors.
    async fn call_model(&mut self, request: ModelRequest) -> Step<ModelResponse> {
        let limits = self.exec.limits;
        let seed = self.exec.id.as_uuid().as_u128() as u64;
        for attempt in 1..=limits.max_model_attempts {
            let span = tracing::info_span!(
                "chat",
                gen_ai.operation.name = "chat",
                gen_ai.request.model = %request.model,
                zorix.attempt = attempt
            );
            let call = self.engine.inner.model.generate(request.clone(), self.control.cancel.clone()).instrument(span);
            let outcome = tokio::time::timeout(Duration::from_millis(limits.model_timeout_ms), call).await;
            let (message, retry_after) = match outcome {
                Ok(Ok(response)) => return Ok(response),
                Ok(Err(ModelError::Cancelled)) => return Err(self.cancelled_or_shutdown().await),
                Ok(Err(ModelError::Permanent(message))) => {
                    return Err(self.fail(Failure::new(FailureKind::Model, message)).await);
                }
                Ok(Err(ModelError::Transient { message, retry_after_ms })) => (message, retry_after_ms),
                Ok(Err(ModelError::Provider { kind, message, retry_after_ms })) if kind.is_temporary() => {
                    (message, retry_after_ms)
                }
                Ok(Err(ModelError::Provider { message, .. })) => {
                    return Err(self.fail(Failure::new(FailureKind::Model, message)).await);
                }
                Err(_) => (format!("The model did not answer within {} ms.", limits.model_timeout_ms), None),
            };
            if attempt == limits.max_model_attempts {
                let why = format!("The model failed {attempt} times in a row. Last error: {message}");
                return Err(self.fail(Failure::new(FailureKind::Model, why)).await);
            }
            let delay = self.engine.inner.config.retry.delay_ms(attempt, retry_after, seed);
            let event = self
                .event(EventKind::ModelRetrying, json!({ "attempt": attempt, "delay_ms": delay, "error": message }));
            self.commit(&[], vec![event], Extras::default()).await?;
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(delay)) => {}
                _ = self.control.cancel.cancelled() => return Err(self.cancelled_or_shutdown().await),
            }
        }
        Err(self.fail(Failure::new(FailureKind::Internal, "The model was never called.")).await)
    }

    async fn final_answer(&mut self, response: ModelResponse, responded: Vec<NewEvent>) -> Step {
        let text = response.text.trim().to_string();
        let structured = self.exec.request.expected_output.as_ref().and_then(|_| parse_structured(&text));
        let result = ExecutionResult { text: text.clone(), structured };

        let mut rejection: Option<(String, String)> = None;
        for validator in &self.engine.inner.validators {
            if let Validation::Reject { reason } = validator.validate(&self.exec, &result).await {
                rejection = Some((validator.name().to_string(), reason));
                break;
            }
        }

        match rejection {
            None => {
                let completed = self.event(
                    EventKind::Completed,
                    json!({
                        "result_chars": text.chars().count(),
                        "iterations": self.exec.iteration,
                        "tool_calls": self.exec.tool_calls_requested,
                        "usage": self.exec.usage,
                    }),
                );
                self.exec.result = Some(result);
                self.state.pending.clear();
                self.commit(
                    &[Trigger::StepFinished, Trigger::Complete],
                    with(responded, completed),
                    Extras::checkpoint(),
                )
                .await?;
                Err(Stop::Done)
            }
            Some((validator, reason)) => {
                let rejected = self.event(
                    EventKind::ResultRejected,
                    json!({ "validator": validator, "reason": reason, "attempt": self.exec.validation_retries + 1 }),
                );
                if self.exec.validation_retries >= self.exec.limits.max_validation_retries {
                    self.commit(&[Trigger::StepFinished], with(responded, rejected), Extras::default()).await?;
                    return Err(self
                        .fail(Failure::new(FailureKind::Validation, format!("The answer was rejected: {reason}")))
                        .await);
                }
                self.exec.validation_retries += 1;
                self.state.transcript.push(Message::assistant(text, Vec::new()).with_native(response.native.clone()));
                self.state
                    .transcript
                    .push(Message::user(format!("Your answer was rejected: {reason} Answer again and fix this.")));
                self.commit(&[Trigger::StepFinished], with(responded, rejected), Extras::checkpoint()).await
            }
        }
    }

    // ---- tools ------------------------------------------------------------------------------------------------

    fn record_for(&self, call: &ToolCallRequest, idempotent: bool) -> ToolCallRecord {
        ToolCallRecord {
            execution_id: self.exec.id,
            call_id: call.id.clone(),
            tool: call.tool.clone(),
            arguments: call.arguments.clone(),
            call_hash: call_hash(&call.tool, &call.arguments),
            status: ToolCallStatus::Requested,
            result: None,
            error: None,
            attempts: 0,
            idempotent,
            requested_at: self.engine.now(),
            started_at: None,
            finished_at: None,
        }
    }

    /// Handle the first pending tool call: validate, decide, maybe suspend for approval, run, record.
    async fn tool_call(&mut self, call: ToolCallRequest) -> Step {
        let tool = self.engine.inner.tools.get(&call.tool);
        let idempotent = tool.as_ref().is_some_and(|t| t.spec().annotations.idempotent);
        let hash = call_hash(&call.tool, &call.arguments);

        if !self.records.iter().any(|r| r.call_id == call.id) {
            self.exec.tool_calls_requested += 1;
            if let Some(failure) = self.engine.inner.policy.check_limits(&self.exec) {
                return Err(self.fail(failure).await);
            }
            let record = self.record_for(&call, idempotent);
            let event = self.event(
                EventKind::ToolRequested,
                json!({ "call_id": call.id, "tool": call.tool, "arguments": call.arguments, "call_hash": hash }),
            );
            self.upsert_record(record.clone());
            self.commit(&[], vec![event], Extras { tool_calls: vec![record], ..Extras::default() }).await?;
        }

        let Some(tool) = tool else {
            let message = format!("There is no tool called {}.", call.tool);
            return self.settle_failed(&call, message, EventKind::ToolDenied, "runtime").await;
        };
        let spec = tool.spec().clone();

        if let Err(error) = ToolExecutor::validate_arguments(&spec, &call.arguments) {
            return self.settle_failed(&call, error.to_string(), EventKind::ToolFailed, "validation").await;
        }

        let decision = self.engine.inner.policy.evaluate_tool_call(&PolicyRequest {
            execution: &self.exec,
            tool: &spec,
            arguments: &call.arguments,
        });
        let existing = self.approvals.iter().find(|a| a.call_id == call.id && a.call_hash == hash).cloned();

        match (decision, existing) {
            (PolicyDecision::Deny { reason }, _) => {
                self.settle_failed(&call, format!("Denied: {reason}"), EventKind::ToolDenied, "policy").await
            }
            (_, Some(approval)) if approval.status == ApprovalStatus::Denied => {
                let note = approval.note.map(|n| format!(" ({n})")).unwrap_or_default();
                self.settle_failed(
                    &call,
                    format!("Denied: a person refused this call{note}."),
                    EventKind::ToolDenied,
                    "person",
                )
                .await
            }
            (_, Some(approval)) if approval.status == ApprovalStatus::Pending => self.wait_for(approval, false).await,
            (_, Some(_)) | (PolicyDecision::Allow, None) => self.run_tool(&call, tool).await,
            (PolicyDecision::RequireApproval { reason }, None) => {
                let approval = Approval {
                    id: ApprovalId::new(),
                    execution_id: self.exec.id,
                    call_id: call.id.clone(),
                    tool: call.tool.clone(),
                    arguments: call.arguments.clone(),
                    call_hash: hash,
                    reason,
                    status: ApprovalStatus::Pending,
                    requested_at: self.engine.now(),
                    decided_at: None,
                    decided_by: None,
                    note: None,
                };
                self.wait_for(approval, true).await
            }
        }
    }

    /// Suspend the execution until a person answers `approval`. The worker lets go of it.
    async fn wait_for(&mut self, approval: Approval, is_new: bool) -> Step {
        let mut record = self.records.iter().find(|r| r.call_id == approval.call_id).cloned();
        if let Some(record) = record.as_mut() {
            record.status = ToolCallStatus::AwaitingApproval;
        }
        if is_new {
            self.approvals.push(approval.clone());
        }
        let waiting = self.event(
            EventKind::WaitingApproval,
            json!({
                "approval_id": approval.id,
                "call_id": approval.call_id,
                "tool": approval.tool,
                "arguments": approval.arguments,
                "call_hash": approval.call_hash,
                "reason": approval.reason,
            }),
        );
        let checkpoint = self.event(EventKind::Checkpoint, json!({ "step": self.step + 1 }));
        let extras = Extras {
            tool_calls: record.iter().cloned().collect(),
            approvals: if is_new { vec![approval] } else { Vec::new() },
            checkpoint: true,
        };
        if let Some(record) = record {
            self.upsert_record(record);
        }
        self.commit(&[Trigger::AwaitApproval], vec![waiting, checkpoint], extras).await?;
        Err(Stop::Done)
    }

    /// The call ends without running: the model gets an error result and the loop goes on.
    async fn settle_failed(&mut self, call: &ToolCallRequest, message: String, kind: EventKind, by: &str) -> Step {
        let now = self.engine.now();
        let status = if kind == EventKind::ToolDenied { ToolCallStatus::Denied } else { ToolCallStatus::Failed };
        let record = self.records.iter().find(|r| r.call_id == call.id).cloned().map(|mut record| {
            record.status = status;
            record.error = Some(message.clone());
            record.finished_at = Some(now);
            record
        });
        self.state.transcript.push(Message::tool_result(call.id.clone(), message.clone(), true));
        self.state.pending.retain(|p| p.id != call.id);
        let event = self.event(kind, json!({ "call_id": call.id, "tool": call.tool, "reason": message, "by": by }));
        let mut extras = Extras::checkpoint();
        if let Some(record) = record {
            self.upsert_record(record.clone());
            extras.tool_calls.push(record);
        }
        self.commit(&[], vec![event], extras).await
    }

    async fn run_tool(&mut self, call: &ToolCallRequest, tool: Arc<dyn Tool>) -> Step {
        let max_attempts = self.engine.inner.config.max_tool_attempts.max(1);
        let spec = tool.spec().clone();
        let seed = self.exec.id.as_uuid().as_u128() as u64;
        loop {
            let Some(mut record) = self.records.iter().find(|r| r.call_id == call.id).cloned() else {
                return Err(Stop::Halt(format!("no record for {}", call.id)));
            };
            let now = self.engine.now();
            record.attempts += 1;
            record.status = ToolCallStatus::Started;
            record.started_at = Some(now);
            self.upsert_record(record.clone());
            let attempt = record.attempts;
            let started = self
                .event(EventKind::ToolStarted, json!({ "call_id": call.id, "tool": call.tool, "attempt": attempt }));
            let triggers: &[Trigger] =
                if self.exec.state == ExecutionState::Running { &[Trigger::AwaitTool] } else { &[] };
            self.commit(triggers, vec![started], Extras { tool_calls: vec![record.clone()], ..Extras::default() })
                .await?;

            let context = ToolContext {
                execution_id: self.exec.id,
                tenant: self.exec.tenant.clone(),
                call_id: call.id.clone(),
                grants: self.exec.grants.clone(),
                cancel: self.control.cancel.clone(),
                workspace_root: self.exec.workspace.clone().or_else(|| self.engine.inner.config.workspace_root.clone()),
                conversation: self.exec.conversation.clone(),
            };
            let began = self.engine.now();
            let span = tracing::info_span!(
                "execute_tool",
                gen_ai.operation.name = "execute_tool",
                gen_ai.tool.name = %call.tool,
                gen_ai.tool.call.id = %call.id,
                zorix.attempt = attempt
            );
            let outcome = ToolExecutor::run(tool.as_ref(), &context, call.arguments.clone()).instrument(span).await;
            let took_ms = self.engine.now().since(began);

            let mut done = record;
            done.finished_at = Some(self.engine.now());
            match outcome {
                Ok(value) => {
                    let text = value.to_string();
                    done.status = ToolCallStatus::Succeeded;
                    done.result = Some(value);
                    self.state.transcript.push(Message::tool_result(call.id.clone(), text.clone(), false));
                    let event = self.event(
                        EventKind::ToolCompleted,
                        json!({ "call_id": call.id, "tool": call.tool, "duration_ms": took_ms, "output_bytes": text.len() }),
                    );
                    return self.finish_tool(call, done, event).await;
                }
                Err(CallError::Cancelled(_)) => return Err(self.cancelled_or_shutdown().await),
                Err(error) => {
                    let may_retry = match &error {
                        CallError::Tool { retryable, .. } => *retryable,
                        CallError::TimedOut { .. } => spec.annotations.idempotent,
                        _ => false,
                    };
                    if may_retry && attempt < max_attempts {
                        let delay = self.engine.inner.config.retry.delay_ms(attempt, None, seed);
                        tokio::select! {
                            _ = tokio::time::sleep(Duration::from_millis(delay)) => {}
                            _ = self.control.cancel.cancelled() => return Err(self.cancelled_or_shutdown().await),
                        }
                        continue;
                    }
                    let message = error.to_string();
                    done.status = ToolCallStatus::Failed;
                    done.error = Some(message.clone());
                    self.state.transcript.push(Message::tool_result(call.id.clone(), message.clone(), true));
                    let event = self.event(
                        EventKind::ToolFailed,
                        json!({ "call_id": call.id, "tool": call.tool, "duration_ms": took_ms, "error": message, "attempts": attempt }),
                    );
                    return self.finish_tool(call, done, event).await;
                }
            }
        }
    }

    async fn finish_tool(&mut self, call: &ToolCallRequest, record: ToolCallRecord, event: NewEvent) -> Step {
        self.state.pending.retain(|p| p.id != call.id);
        self.exec.usage.tool_calls += 1;
        self.upsert_record(record.clone());
        let extras = Extras { tool_calls: vec![record], checkpoint: true, ..Extras::default() };
        self.commit(&[Trigger::StepFinished], vec![event], extras).await
    }
}
