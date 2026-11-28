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
    Approval, ApprovalId, ApprovalStatus, Checkpoint, Commit, DelegateRequest, Error, EventKind, Execution,
    ExecutionEngine, ExecutionId, ExecutionResult, ExecutionState, ExecutionView, Failure, FailureKind, FinishReason,
    LimitKind, LoopState, Message, ModelError, ModelRequest, ModelResponse, NewEvent, Permission, PolicyDecision,
    PolicyRequest, Role, StepPhase, Timestamp, TokenSample, Tool, ToolCallId, ToolCallRecord, ToolCallRequest,
    ToolCallStatus, ToolContext, ToolSpec, Trigger, Validation, call_hash, child_key,
};
use zorix_core::{MAX_DEPTH, MAX_DESCENDANTS, PlanContext};

use crate::engine::{Engine, RunControl};
use crate::tools::{CallError, ToolExecutor};
use crate::validators::parse_structured;
use crate::watchdog::{MODEL_CALL_CEILING_MS, Progress, Timeout, Watched, guard};

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

/// Children that were just created for a delegate call: what the record keeps about them, and the executions themselves.
type Fresh = (Vec<Value>, Vec<(zorix_core::DelegateTask, Execution)>);

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
    /// Summarising failed once in this drive; it is not tried again until the run is picked up again.
    summaries_failed: bool,
    engine: Engine,
    exec: Execution,
    state: LoopState,
    step: u64,
    control: Arc<RunControl>,
    tick: Timestamp,
    /// Time a tool call spent waiting for the person, not yet taken off the active time.
    waited_ms: u64,
    records: Vec<ToolCallRecord>,
    approvals: Vec<Approval>,
}

impl Engine {
    /// Work on one queued execution until it finishes, fails, is cancelled, pauses or needs a person. One worker at a time: a run that has
    /// just stopped for a person and is answered at once can be picked up while the worker that stopped it is still cleaning up, and the
    /// clean-up releases the lease and forgets the run's control, which the new worker would then be missing.
    pub(crate) async fn drive(&self, id: ExecutionId) {
        if !self.inner.driving.lock().insert(id) {
            return self.try_again_later(id);
        }
        struct Done<'a>(&'a Engine, ExecutionId);
        impl Drop for Done<'_> {
            fn drop(&mut self) {
                self.0.inner.driving.lock().remove(&self.1);
            }
        }
        let _done = Done(self, id);
        self.drive_alone(id).await;
    }

    async fn drive_alone(&self, id: ExecutionId) {
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
            lost: std::sync::atomic::AtomicBool::new(false),
        });
        inner.runs.lock().insert(id, control.clone());
        let heartbeat = self.heartbeat(id, control.clone());

        // Span names and attributes follow the OpenTelemetry GenAI conventions, so an OTLP exporter can be added later
        // without changing the runtime. Nothing a person or a model wrote (prompts, arguments, results) goes in them.
        let span = tracing::info_span!(
            "invoke_agent",
            gen_ai.operation.name = "invoke_agent",
            zorix.execution.id = %id,
            zorix.execution.state = tracing::field::Empty,
            gen_ai.usage.input_tokens = tracing::field::Empty,
            gen_ai.usage.output_tokens = tracing::field::Empty,
            zorix.usage.cost_micros = tracing::field::Empty,
            zorix.usage.model_calls = tracing::field::Empty,
            zorix.usage.tool_calls = tracing::field::Empty,
            otel.status_code = tracing::field::Empty,
        );
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
        drive.instrument(span.clone()).await;
        // What the run came to, as numbers and a state, for whoever reads the spans (never what was said).
        if let Ok(Some(done)) = inner.repository.load(id).await {
            span.record("zorix.execution.state", done.state.as_str());
            span.record("gen_ai.usage.input_tokens", done.usage.input_tokens);
            span.record("gen_ai.usage.output_tokens", done.usage.output_tokens);
            span.record("zorix.usage.cost_micros", done.usage.cost_micros);
            span.record("zorix.usage.model_calls", u64::from(done.usage.model_calls));
            span.record("zorix.usage.tool_calls", u64::from(done.usage.tool_calls));
            if done.state == ExecutionState::Failed {
                span.record("otel.status_code", "ERROR");
            }
        }

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

    /// Keep the lease alive while a long model or tool call runs. When it cannot be kept (another engine holds it, or the store has not
    /// answered for a whole lease) the run is told to stop and to write nothing more: the other engine may be driving the same run.
    fn heartbeat(&self, id: ExecutionId, control: Arc<RunControl>) -> CancellationToken {
        let stop = CancellationToken::new();
        let (engine, token) = (self.clone(), stop.clone());
        tokio::spawn(async move {
            let ttl = engine.inner.config.lease_ttl_ms;
            let every = Duration::from_millis((ttl / 3).max(50));
            let mut renewed_at = engine.now();
            let lose = |why: &str| {
                tracing::warn!(execution = %id, why, "the lease was lost; the run stops");
                control.lost.store(true, Ordering::SeqCst);
                control.cancel.cancel();
            };
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
                match renewed {
                    Ok(true) => renewed_at = engine.now(),
                    Ok(false) => return lose("another engine holds it"),
                    Err(error) => {
                        tracing::warn!(execution = %id, %error, "could not renew the lease");
                        // Not renewed for a whole lease: another engine may have taken it, and this one cannot tell.
                        if engine.now().since(renewed_at) >= ttl {
                            return lose("it could not be renewed for a whole lease");
                        }
                    }
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
        Ok(Some(Run {
            summaries_failed: false,
            engine,
            exec,
            state,
            step,
            control,
            tick,
            waited_ms: 0,
            records,
            approvals,
        }))
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
        commit.lease_owner = Some(self.engine.inner.owner.clone());
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
        // Waiting for the person is not the run working.
        let waited = std::mem::take(&mut self.waited_ms);
        self.exec.active_ms = self.exec.active_ms.saturating_add(now.since(self.tick).saturating_sub(waited));
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
        // Not a cancellation by a person: the lease went to another engine, which carries on. Nothing is written.
        if self.control.lost.load(Ordering::SeqCst) {
            return Stop::Abandoned;
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

    /// What the model is shown of the history: the transcript, with a summary in place of the messages it stands for.
    fn shown_transcript(&self) -> Vec<Message> {
        match &self.state.summary {
            Some(summary) => {
                let mut shown = vec![crate::history_summary::summary_message(&summary.text)];
                shown.extend(self.state.transcript.iter().skip(summary.covers).cloned());
                shown
            }
            None => self.state.transcript.clone(),
        }
    }

    /// The request for the next model call, from the sources of context.
    async fn build_request(&self, tools: &[ToolSpec], context_budget: Option<u64>) -> zorix_core::Result<ModelRequest> {
        let shown = self.shown_transcript();
        let view = ExecutionView {
            execution: &self.exec,
            transcript: &shown,
            tools,
            context_budget,
            token_factor: self.engine.inner.calibration.factor(&self.exec.request.model),
        };
        let mut request = self.engine.inner.context.build(&view).await?;
        request.phase = self.phase();
        if let Some(agent) = &self.exec.agent {
            request.allowed_models = agent.allowed_models.clone();
        }
        Ok(request)
    }

    /// Ask a model to summarise the oldest turns that `request` had to leave out, and keep the summary in the loop's state. Returns whether
    /// there is a new summary, so the request is built again. It never fails the run: when the summary cannot be made (the setting is off, a
    /// limit is reached, the call failed or answered nothing) the turns stay left out, as before, and it is not tried again in this drive.
    async fn summarise(&mut self, request: &ModelRequest) -> Step<bool> {
        use crate::history_summary as hs;
        if !self.engine.inner.settings.get().context.summaries || self.summaries_failed {
            return Ok(false);
        }
        // Which turns of what was shown were left out: the history segments of the report are the turns, in order.
        let shown = self.shown_transcript();
        let turns = hs::turns(&shown);
        let kept: Vec<bool> = request.context_report.iter().filter(|r| r.origin == "history").map(|r| r.kept).collect();
        if kept.len() != turns.len() {
            return Ok(false);
        }
        let from_shown = usize::from(self.state.summary.is_some());
        let Some(end_shown) = kept
            .iter()
            .enumerate()
            .filter(|(i, kept)| !**kept && *i >= from_shown)
            .map(|(i, _)| turns[i].end)
            .next_back()
        else {
            return Ok(false);
        };
        // In transcript terms: what the summary already covers, and where the newly left out turns end.
        let start = self.state.summary.as_ref().map_or(0, |s| s.covers);
        let end = start + end_shown.saturating_sub(from_shown);
        if end <= start || end > self.state.transcript.len() {
            return Ok(false);
        }
        // A run that is at its limit says so at the next boundary; do not spend more here.
        if self.engine.inner.policy.check_limits(&self.exec).is_some() {
            return Ok(false);
        }
        let previous = self.state.summary.as_ref().map(|s| s.text.clone());
        let ask = hs::request(
            &self.exec.request.model,
            self.exec.id,
            previous.as_deref(),
            &self.state.transcript[start..end],
        );
        let limits = self.exec.limits;
        let call = self.engine.inner.model.generate(ask, self.control.cancel.clone());
        let outcome = tokio::time::timeout(Duration::from_millis(limits.model_timeout_ms), call).await;
        let response = match outcome {
            Ok(Ok(response)) => response,
            Ok(Err(ModelError::Cancelled)) => return Err(self.cancelled_or_shutdown().await),
            Ok(Err(error)) => {
                tracing::warn!(%error, "could not summarise the earlier turns; they are left out");
                self.summaries_failed = true;
                return Ok(false);
            }
            Err(_) => {
                tracing::warn!("summarising the earlier turns took too long; they are left out");
                self.summaries_failed = true;
                return Ok(false);
            }
        };
        // What it cost is the run's, whatever came of it.
        let mut usage = response.usage;
        usage.model_calls = usage.model_calls.max(1);
        usage.tool_calls = 0;
        self.exec.usage.add(&usage);
        let Some(text) = hs::finish(&response.text) else {
            self.summaries_failed = true;
            return Ok(false);
        };
        self.state.summary = Some(zorix_core::HistorySummary { covers: end, text: text.clone() });
        let event = self.event(
            EventKind::ContextSummarised,
            json!({
                "folded_messages": end - start,
                "covers": end,
                "summary_chars": text.chars().count(),
                "input_tokens": usage.input_tokens,
                "output_tokens": usage.output_tokens,
                "cost_micros": usage.cost_micros,
            }),
        );
        self.commit(&[], vec![event], Extras::checkpoint()).await?;
        Ok(true)
    }

    /// The tools this execution may use. They are pinned the first time they are needed and never change afterwards,
    /// so every step and every resume offers the same list, and a server that changes a tool in the meantime cannot
    /// change what a running execution was shown or approved (ADR-0020).
    fn pin_tools(&mut self) {
        if self.exec.tool_pins.is_some() {
            return;
        }
        let pins = self
            .engine
            .inner
            .tools
            .list()
            .into_iter()
            .filter(|spec| self.exec.grants.allows(&Permission::use_tool(&spec.id)))
            .map(|spec| spec.pin())
            .collect();
        self.exec.tool_pins = Some(pins);
    }

    fn offered_tools(&self) -> Vec<ToolSpec> {
        self.exec.tool_pins.iter().flatten().map(|pin| pin.spec.clone()).collect()
    }

    async fn model_turn(&mut self) -> Step {
        self.exec.iteration += 1;
        if let Some(failure) = self.engine.inner.policy.check_limits(&self.exec) {
            return Err(self.fail(failure).await);
        }
        // A cost limit counts what each call costs, and a model with no known price costs "zero" every time: the limit would never
        // trip. Say so before anything is sent (audit finding F11). A model that is not described is not refused: nothing says it is unpriced.
        if self.exec.limits.max_cost_micros.is_some()
            && let Some(profile) = self.engine.inner.model.describe(&self.exec.request.model)
            && !profile.priced
        {
            let message = format!(
                "A cost limit is set, but the price of {} is not known, so the limit could not be kept and nothing was sent. Add the model's price to the provider configuration (providers.toml), or run without a cost limit.",
                self.exec.request.model
            );
            return Err(self.fail(Failure::limit(LimitKind::Cost, message)).await);
        }
        self.pin_tools();
        let tools = self.offered_tools();
        let context_budget = self.engine.inner.model.describe(&self.exec.request.model).map(|profile| {
            profile
                .context_window
                .saturating_sub(profile.max_output)
                .saturating_sub(CONTEXT_MARGIN_TOKENS)
                .max(MIN_CONTEXT_TOKENS)
        });
        let mut request = match self.build_request(&tools, context_budget).await {
            Ok(request) => request,
            Err(error) => {
                return Err(self
                    .fail(Failure::new(FailureKind::Internal, format!("Could not build the context: {error}")))
                    .await);
            }
        };
        // Turns that no longer fit are summarised, when that is on, and the request is built again with the summary where they were.
        if self.summarise(&request).await? {
            request = match self.build_request(&tools, context_budget).await {
                Ok(request) => request,
                Err(error) => {
                    return Err(self
                        .fail(Failure::new(FailureKind::Internal, format!("Could not build the context: {error}")))
                        .await);
                }
            };
        }
        // The raw estimate (before any correction), so the ratio to the real count stays comparable across calls.
        let estimated_messages: u64 = request.messages.iter().map(Message::estimated_tokens).sum();
        let estimated_tools = request.estimated_tool_tokens();
        let phase = request.phase;
        let token_factor = self.engine.inner.calibration.factor(&self.exec.request.model);
        let requested = self.event(
            EventKind::ModelRequested,
            json!({
                "iteration": self.exec.iteration,
                "model": request.model,
                "messages": request.messages.len(),
                "tools": request.tools.len(),
                "token_factor": token_factor,
                "estimated_input_tokens": estimated_messages,
                "estimated_tool_tokens": request.estimated_tool_tokens(),
                "history_edited": request.history_edited,
                "segments": request.context_report,
            }),
        );
        self.commit(&[Trigger::AwaitModel], vec![requested], Extras::default()).await?;

        let allowed = request.allowed_models.clone();
        let response = self.call_model(request).await?;
        // The router never offers a model outside the list. This is the second lock, for any gateway that is not the router.
        if !zorix_core::model_allowed(&allowed, response.provider.as_deref(), &response.model) {
            let message = format!(
                "The model {} answered, but this agent may not use it (allowed: {}).",
                response.model,
                allowed.join(", ")
            );
            return Err(self.fail(Failure::new(FailureKind::Model, message)).await);
        }
        self.engine.inner.calibration.observe(TokenSample {
            requested: self.exec.request.model.clone(),
            answered: response.provider.as_ref().map(|provider| format!("{provider}/{}", response.model)),
            estimated: estimated_messages + estimated_tools,
            actual: response.usage.input_tokens + response.usage.cache_read_tokens + response.usage.cache_write_tokens,
            phase,
        });
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
                zorix.attempt = attempt,
                gen_ai.response.model = tracing::field::Empty,
                gen_ai.usage.input_tokens = tracing::field::Empty,
                gen_ai.usage.output_tokens = tracing::field::Empty,
                zorix.usage.cost_micros = tracing::field::Empty,
                gen_ai.response.finish_reasons = tracing::field::Empty,
                otel.status_code = tracing::field::Empty,
            );
            let sink = self.engine.inner.live.sink(self.exec.id);
            let progress = Progress::new();
            let call = self
                .engine
                .inner
                .model
                .generate_live(
                    request.clone(),
                    self.control.cancel.clone(),
                    Watched::new(sink.clone(), progress.clone()),
                )
                .instrument(span.clone());
            let outcome = guard(&progress, limits.model_timeout_ms, MODEL_CALL_CEILING_MS, call).await;
            let (message, retry_after) = match outcome {
                Ok(Ok(response)) => {
                    span.record("gen_ai.response.model", response.model.as_str());
                    span.record("gen_ai.usage.input_tokens", response.usage.input_tokens);
                    span.record("gen_ai.usage.output_tokens", response.usage.output_tokens);
                    span.record("zorix.usage.cost_micros", response.usage.cost_micros);
                    span.record(
                        "gen_ai.response.finish_reasons",
                        format!("{:?}", response.finish).to_ascii_lowercase(),
                    );
                    return Ok(response);
                }
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
                Err(Timeout::NoProgress) => {
                    (format!("The model did not answer within {} ms.", limits.model_timeout_ms), None)
                }
                Err(Timeout::Ceiling) => (
                    format!(
                        "The model was still answering after {} ms, the most one call may take.",
                        MODEL_CALL_CEILING_MS
                    ),
                    None,
                ),
            };
            // Whatever the failed attempt wrote is not part of the answer: readers of the live text start again.
            sink.reset();
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

        // A provider that declined has not given an answer, and an answer that ran out of room is not a complete one: neither becomes
        // the result of the execution just because it is text.
        if response.finish == FinishReason::Refusal {
            let shown: String = text.chars().take(300).collect();
            let message = if shown.is_empty() {
                "The model declined to answer.".to_string()
            } else {
                format!("The model declined to answer: {shown}")
            };
            self.commit(&[Trigger::StepFinished], responded, Extras::default()).await?;
            return Err(self.fail(Failure::new(FailureKind::Model, message)).await);
        }
        let mut rejection: Option<(String, String)> = (response.finish == FinishReason::Length).then(|| {
            (
                "model".to_string(),
                "The answer was cut off because the model ran out of room. Answer again, shorter, and finish it."
                    .to_string(),
            )
        });
        if rejection.is_none() {
            for validator in &self.engine.inner.validators {
                if let Validation::Reject { reason } = validator.validate(&self.exec, &result).await {
                    rejection = Some((validator.name().to_string(), reason));
                    break;
                }
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
                    let kind = if validator == "model" { FailureKind::Model } else { FailureKind::Validation };
                    return Err(self.fail(Failure::new(kind, format!("The answer was rejected: {reason}"))).await);
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
                json!({
                    "call_id": call.id,
                    "tool": call.tool,
                    "arguments": call.arguments,
                    "call_hash": hash,
                    "source": tool.as_ref().map(|t| t.spec().source.label()),
                }),
            );
            self.upsert_record(record.clone());
            self.commit(&[], vec![event], Extras { tool_calls: vec![record], ..Extras::default() }).await?;
        }

        // A delegated call comes back here when its children have finished (or when the run was queued again by mistake).
        if call.tool == zorix_core::delegation::TOOL
            && let Some(record) = self.records.iter().find(|r| r.call_id == call.id).cloned()
        {
            match record.status {
                ToolCallStatus::Succeeded => return self.deliver_delegation(&call, record).await,
                ToolCallStatus::Delegated => return self.suspend_for_children(&call, None).await,
                _ => {}
            }
        }

        let Some(tool) = tool else {
            let message = format!("There is no tool called {}.", call.tool);
            return self.settle_failed(&call, message, EventKind::ToolDenied, "runtime").await;
        };
        let spec = tool.spec().clone();

        // A tool the execution was not shown, or whose definition changed since it was, does not run.
        if let Some(pins) = &self.exec.tool_pins {
            match pins.iter().find(|pin| pin.spec.id == call.tool) {
                // Not granted at all: the policy below gives the usual "no permission" answer.
                None if !self.exec.grants.allows(&Permission::use_tool(&call.tool)) => {}
                None => {
                    let message = format!("{} was not available to this execution when it started.", call.tool);
                    return self.settle_failed(&call, message, EventKind::ToolDenied, "runtime").await;
                }
                Some(pin) if pin.hash != spec.definition_hash() => {
                    let message = format!(
                        "{} changed after this execution started (its definition is not the one it was shown), so it was not run.",
                        call.tool
                    );
                    return self.settle_failed(&call, message, EventKind::ToolDenied, "runtime").await;
                }
                Some(_) => {}
            }
        }

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
            (PolicyDecision::Suggest { reason }, _) => {
                self.settle_failed(&call, reason, EventKind::ToolSuggested, "autonomy").await
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
            (PolicyDecision::AllowUnattended { reason }, None) => {
                let level = self.exec.agent.as_ref().and_then(|a| a.autonomy.as_ref()).map(|a| a.level);
                let event = self.event(
                    EventKind::ToolUnattended,
                    json!({
                        "call_id": call.id,
                        "tool": call.tool,
                        "arguments": call.arguments,
                        "call_hash": hash,
                        "level": level,
                        "reason": reason,
                        "summary": self.engine.inner.tools.get(&call.tool).and_then(|t| t.describe_call(&call.arguments)),
                    }),
                );
                self.commit(&[], vec![event], Extras::default()).await?;
                if call.tool == zorix_core::delegation::TOOL {
                    self.delegate(&call).await
                } else {
                    self.run_tool(&call, tool).await
                }
            }
            (_, Some(_)) | (PolicyDecision::Allow, None) if call.tool == zorix_core::delegation::TOOL => {
                self.delegate(&call).await
            }
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
                "person_only": self.engine.inner.tools.get(&approval.tool).is_some_and(|t| t.spec().approval_by_person_only),
                "source": self.engine.inner.tools.get(&approval.tool).map(|t| t.spec().source.label()),
                "definition_hash": self.exec.tool_pins.iter().flatten().find(|p| p.spec.id == approval.tool).map(|p| p.hash.clone()),
                "summary": self.engine.inner.tools.get(&approval.tool).and_then(|t| t.describe_call(&approval.arguments)),
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
        let status = if matches!(kind, EventKind::ToolDenied | EventKind::ToolSuggested) {
            ToolCallStatus::Denied
        } else {
            ToolCallStatus::Failed
        };
        let record = self.records.iter().find(|r| r.call_id == call.id).cloned().map(|mut record| {
            record.status = status;
            record.error = Some(message.clone());
            record.finished_at = Some(now);
            record
        });
        self.state.transcript.push(Message::tool_result(call.id.clone(), message.clone(), true));
        self.state.pending.retain(|p| p.id != call.id);
        let mut payload = json!({ "call_id": call.id, "tool": call.tool, "reason": message, "by": by });
        if kind == EventKind::ToolSuggested {
            // The proposal itself: what the person is asked to do, in the exact arguments.
            payload["arguments"] = call.arguments.clone();
            payload["summary"] =
                json!(self.engine.inner.tools.get(&call.tool).and_then(|t| t.describe_call(&call.arguments)));
        }
        let event = self.event(kind, payload);
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

            let host = crate::asks::RunHost::new(
                self.engine.inner.clone(),
                crate::asks::CallScope {
                    tenant: self.exec.tenant.clone(),
                    execution_id: self.exec.id,
                    call_id: call.id.clone(),
                    model: self.exec.request.model.clone(),
                    allowed_models: self.exec.agent.as_ref().map(|a| a.allowed_models.clone()).unwrap_or_default(),
                    tokens_left: self
                        .exec
                        .limits
                        .max_total_tokens
                        .map(|max| max.saturating_sub(self.exec.usage.total_tokens())),
                    spent: self.exec.limits.max_cost_micros.is_some_and(|max| self.exec.usage.cost_micros >= max),
                    model_timeout: Duration::from_millis(self.exec.limits.model_timeout_ms),
                },
                &self.control.cancel,
            );
            let context = ToolContext {
                execution_id: self.exec.id,
                tenant: self.exec.tenant.clone(),
                call_id: call.id.clone(),
                grants: self.exec.grants.clone(),
                cancel: self.control.cancel.clone(),
                workspace_root: self.exec.workspace.clone().or_else(|| self.engine.inner.config.workspace_root.clone()),
                conversation: self.exec.conversation.clone(),
                agent: self.exec.agent.clone(),
                host: Some(host.clone()),
            };
            let began = self.engine.now();
            let span = tracing::info_span!(
                "execute_tool",
                gen_ai.operation.name = "execute_tool",
                gen_ai.tool.name = %call.tool,
                gen_ai.tool.call.id = %call.id,
                zorix.attempt = attempt,
                zorix.tool.outcome = tracing::field::Empty,
                otel.status_code = tracing::field::Empty,
            );
            let outcome =
                ToolExecutor::run(tool.as_ref(), &context, call.arguments.clone()).instrument(span.clone()).await;
            match &outcome {
                Ok(_) => span.record("zorix.tool.outcome", "ok"),
                Err(CallError::Cancelled(_)) => span.record("zorix.tool.outcome", "cancelled"),
                Err(_) => span.record("zorix.tool.outcome", "failed").record("otel.status_code", "ERROR"),
            };
            let took_ms = self.engine.now().since(began);
            // What a server's model call cost is the run's, whatever came of the tool call.
            let (asked_usage, asked) = host.close();
            self.exec.usage.add(&asked_usage);
            let waited_ms = u64::try_from(zorix_core::CallHost::waited(&*host).as_millis()).unwrap_or(u64::MAX);
            self.waited_ms = self.waited_ms.saturating_add(waited_ms);
            let took_ms = took_ms.saturating_sub(waited_ms);

            let mut done = record;
            done.finished_at = Some(self.engine.now());
            match outcome {
                Ok(value) => {
                    let text = value.to_string();
                    done.status = ToolCallStatus::Succeeded;
                    done.result = Some(value);
                    self.state.transcript.push(Message::tool_result(call.id.clone(), text.clone(), false));
                    let mut payload = json!({ "call_id": call.id, "tool": call.tool, "duration_ms": took_ms, "output_bytes": text.len() });
                    if !asked.is_empty() {
                        payload["server_requests"] = json!(asked);
                    }
                    let event = self.event(EventKind::ToolCompleted, payload);
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
                    let mut payload = json!({ "call_id": call.id, "tool": call.tool, "duration_ms": took_ms, "error": message, "attempts": attempt });
                    if !asked.is_empty() {
                        payload["server_requests"] = json!(asked);
                    }
                    let event = self.event(EventKind::ToolFailed, payload);
                    return self.finish_tool(call, done, event).await;
                }
            }
        }
    }

    /// Hand the work to other executions and suspend until they finish (ADR-0029). The worker is free while they work.
    async fn delegate(&mut self, call: &ToolCallRequest) -> Step {
        let request = match DelegateRequest::parse(&call.arguments) {
            Ok(request) => request,
            Err(message) => return self.settle_failed(call, message, EventKind::ToolFailed, "validation").await,
        };
        let Some(planner) = self.engine.inner.planner.clone() else {
            let message = "Delegation is not set up in this program.".to_string();
            return self.settle_failed(call, message, EventKind::ToolFailed, "runtime").await;
        };
        let repository = self.engine.inner.repository.clone();
        let root = self.exec.root.unwrap_or(self.exec.id);
        if self.exec.depth >= MAX_DEPTH {
            let message =
                format!("Delegation is nested {MAX_DEPTH} levels deep at most, and this is the deepest level.");
            return self.settle_failed(call, message, EventKind::ToolFailed, "delegation").await;
        }
        let mut ancestors = Vec::new();
        let mut next = self.exec.parent;
        while let Some(id) = next {
            if ancestors.len() > MAX_DEPTH as usize + 1 {
                break;
            }
            match repository.load(id).await? {
                Some(parent) => {
                    next = parent.parent;
                    ancestors.push(parent);
                }
                None => break,
            }
        }
        let descendants = repository.descendants(&self.exec.tenant, root).await?.len();
        if descendants + request.tasks.len() > MAX_DESCENDANTS {
            let message = format!(
                "This work already has {descendants} sub-runs and at most {MAX_DESCENDANTS} are allowed, so {} more cannot start.",
                request.tasks.len()
            );
            return self.settle_failed(call, message, EventKind::ToolFailed, "delegation").await;
        }
        let context = PlanContext { parent: &self.exec, call_id: call.id.0.as_str(), ancestors, descendants };
        let planned = match planner.plan(context, &request).await {
            Ok(planned) if planned.len() == request.tasks.len() => planned,
            Ok(_) => return Err(Stop::Halt("the planner returned a different number of tasks".into())),
            Err(why) => return self.settle_failed(call, why, EventKind::ToolFailed, "delegation").await,
        };

        // Create the children. The keys make this safe to repeat: after a crash the same children are found again.
        let mut children = Vec::with_capacity(planned.len());
        for (index, mut child) in planned.into_iter().enumerate() {
            child.new.tenant = Some(self.exec.tenant.clone());
            child.new.parent = Some(self.exec.id);
            child.new.root = Some(root);
            child.new.depth = self.exec.depth + 1;
            child.new.conversation = None;
            child.new.idempotency_key = Some(child_key(&self.exec.id.to_string(), call.id.0.as_str(), index));
            match self.engine.create(child.new).await {
                Ok(created) => children.push((child.task, created)),
                Err(error) => {
                    let started: Vec<ExecutionId> = children.iter().map(|(_, c)| c.id).collect();
                    for id in started {
                        let _ = self
                            .engine
                            .cancel(&self.exec.tenant, id, Some("The delegation could not be completed.".into()))
                            .await;
                    }
                    let message = format!("The task for {} could not be started: {error}", child.task.agent);
                    return self.settle_failed(call, message, EventKind::ToolFailed, "delegation").await;
                }
            }
        }
        let delegated: Vec<Value> = children
            .iter()
            .map(|(task, child)| {
                json!({ "child": child.id, "agent": task.agent, "task": task.task.chars().take(200).collect::<String>() })
            })
            .collect();
        self.suspend_for_children(call, Some((delegated, children))).await
    }

    /// Commit "waiting for children", then start them. With `None` (a call that was already delegated) it only suspends again.
    async fn suspend_for_children(&mut self, call: &ToolCallRequest, fresh: Option<Fresh>) -> Step {
        let mut record = self.records.iter().find(|r| r.call_id == call.id).cloned();
        let mut events = Vec::new();
        if let Some((delegated, children)) = &fresh {
            if let Some(record) = record.as_mut() {
                record.status = ToolCallStatus::Delegated;
                record.result = Some(json!({ "delegated": delegated }));
            }
            let ids: Vec<ExecutionId> = children.iter().map(|(_, c)| c.id).collect();
            events.push(self.event(
                EventKind::WaitingChildren,
                json!({ "call_id": call.id, "children": ids, "tasks": children.len() }),
            ));
            for (task, child) in children {
                events.push(self.event(
                    EventKind::ChildStarted,
                    json!({
                        "child": child.id, "agent": task.agent, "depth": child.depth,
                        "task": task.task.chars().take(200).collect::<String>(),
                    }),
                ));
            }
            events.push(self.event(EventKind::Checkpoint, json!({ "step": self.step + 1 })));
        } else {
            events.push(
                self.event(EventKind::WaitingChildren, json!({ "call_id": call.id, "children": [], "again": true })),
            );
        }
        let extras =
            Extras { tool_calls: record.iter().cloned().collect(), checkpoint: fresh.is_some(), ..Extras::default() };
        if let Some(record) = record {
            self.upsert_record(record);
        }
        // The parent is suspended before any child starts, so a child that finishes at once finds it waiting.
        self.commit(&[Trigger::AwaitChildren], events, extras).await?;
        if let Some((_, children)) = fresh {
            for (_, child) in children {
                let current = self.engine.get(&self.exec.tenant, child.id).await?;
                if current.state == ExecutionState::Created {
                    self.engine.start(&self.exec.tenant, child.id).await?;
                }
            }
        }
        // Children that were already finished (a repeat after a crash) must not leave the parent waiting for nothing.
        self.engine.reconcile_children(self.exec.id).await?;
        Err(Stop::Done)
    }

    /// The children finished and the engine wrote their results into the record: the model gets them as the tool's result.
    async fn deliver_delegation(&mut self, call: &ToolCallRequest, record: ToolCallRecord) -> Step {
        let text = record.result.as_ref().map(Value::to_string).unwrap_or_else(|| "{}".into());
        self.state.transcript.push(Message::tool_result(call.id.clone(), text.clone(), false));
        let event = self.event(
            EventKind::ToolCompleted,
            json!({ "call_id": call.id, "tool": call.tool, "duration_ms": 0, "output_bytes": text.len() }),
        );
        self.finish_tool(call, record, event).await
    }

    async fn finish_tool(&mut self, call: &ToolCallRequest, record: ToolCallRecord, event: NewEvent) -> Step {
        self.state.pending.retain(|p| p.id != call.id);
        self.exec.usage.tool_calls += 1;
        self.upsert_record(record.clone());
        let extras = Extras { tool_calls: vec![record], checkpoint: true, ..Extras::default() };
        // A tool that ran left the state at WaitingForTool. A delegated call comes back with the run just started, so it passes
        // through that state in the same commit.
        let triggers: &[Trigger] = if self.exec.state == ExecutionState::Running {
            &[Trigger::AwaitTool, Trigger::StepFinished]
        } else {
            &[Trigger::StepFinished]
        };
        self.commit(triggers, vec![event], extras).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use zorix_core::{ExecutionEngine, ExecutionState, NewExecution, TaskRequest, TenantId};
    use zorix_sim::{ScriptedModel, Turn};

    use crate::{Engine, EngineConfig, MemoryRepository};

    /// An execution that two workers pick up at once (a run that has just stopped for a person and is answered at once is the usual way):
    /// the one that finishes its clean-up must not release the lease the other holds, or the other's next commit is refused as coming
    /// from an engine without the lease and the run is left unfinished.
    #[tokio::test]
    async fn two_drives_of_one_execution_do_not_take_each_others_lease() {
        let model = Arc::new(ScriptedModel::new([Turn::Delay { ms: 150, then: Box::new(Turn::Say("done".into())) }]));
        let engine = Engine::builder(Arc::new(MemoryRepository::new()), model.clone())
            .config(EngineConfig { workers: 2, lease_ttl_ms: 1_000, ..EngineConfig::default() })
            .build();
        let new = NewExecution {
            request: TaskRequest { prompt: "go".into(), model: "sim".into(), ..TaskRequest::default() },
            ..NewExecution::default()
        };
        let started = engine.submit(new).await.unwrap();
        // The engine's own worker drives it too; two more drives by hand overlap with it and with each other.
        tokio::join!(engine.drive(started.id), engine.drive(started.id));
        let done = tokio::time::timeout(Duration::from_secs(10), engine.settle(&TenantId::local(), started.id))
            .await
            .expect("the execution settles")
            .unwrap();
        assert_eq!(done.state, ExecutionState::Completed, "{:?}", done.failure);
        assert_eq!(model.requests().len(), 1, "the model was called once, not once per drive");
    }
}
