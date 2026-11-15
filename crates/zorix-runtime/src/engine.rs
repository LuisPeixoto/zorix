//! The engine: creates, controls and recovers executions. The loop that drives one execution is in `run.rs`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::FutureExt;
use futures_util::stream::BoxStream;
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use zorix_core::{
    Approval, ApprovalDecision, ApprovalId, ApprovalStatus, Clock, Commit, ContextBuilder, CreateOutcome, Error,
    EventKind, EventPublisher, Execution, ExecutionDetail, ExecutionEngine, ExecutionEvent, ExecutionId,
    ExecutionRepository, ExecutionScheduler, ExecutionState, InvalidTransition, ListFilter, ModelGateway, NewEvent,
    NewExecution, PolicyEngine, RecoveryReport, Result, ResultValidator, SystemClock, TenantId, Timestamp,
    ToolCallRecord, ToolCallStatus, ToolRegistry, Trigger,
};

use crate::context::DefaultContextBuilder;
use crate::events::{EventBus, follow};
use crate::policy::DefaultPolicy;
use crate::redact::redact;
use crate::retry::RetryPolicy;
use crate::scheduler::LocalScheduler;
use crate::tools::InMemoryToolRegistry;
use crate::validators::{NonEmptyValidator, OutputSchemaValidator};

/// Settings of an [`Engine`].
#[derive(Clone, Debug)]
pub struct EngineConfig {
    /// How many executions run at the same time.
    pub workers: usize,
    /// How long a worker's claim on an execution lasts without a heartbeat, in milliseconds.
    pub lease_ttl_ms: u64,
    /// Waiting between retries of model and tool calls.
    pub retry: RetryPolicy,
    /// How many times one tool call is started when it fails with a temporary error.
    pub max_tool_attempts: u32,
    /// The folder file tools are confined to. `None` means tools get no workspace.
    pub workspace_root: Option<PathBuf>,
    /// The host guarantees no other process uses this store (for example it holds the data folder's lock). Recovery
    /// then drops the leases that crashed processes left behind and takes over their work at once. Leave it off when
    /// several engines may share a store: recovery then skips whatever a live engine still holds.
    pub sole_owner: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            workers: 4,
            lease_ttl_ms: 30_000,
            retry: RetryPolicy::default(),
            max_tool_attempts: 3,
            workspace_root: None,
            sole_owner: false,
        }
    }
}

/// What a worker shares with the API while it drives one execution.
pub(crate) struct RunControl {
    pub cancel: CancellationToken,
    pub reason: Mutex<Option<String>>,
    pub pause: AtomicBool,
}

pub(crate) struct Inner {
    pub repository: Arc<dyn ExecutionRepository>,
    pub model: Arc<dyn ModelGateway>,
    pub tools: Arc<dyn ToolRegistry>,
    pub policy: Arc<dyn PolicyEngine>,
    pub context: Arc<dyn ContextBuilder>,
    pub scheduler: Arc<dyn ExecutionScheduler>,
    pub clock: Arc<dyn Clock>,
    pub bus: EventBus,
    pub publishers: Vec<Arc<dyn EventPublisher>>,
    pub validators: Vec<Arc<dyn ResultValidator>>,
    pub config: EngineConfig,
    pub runs: Mutex<HashMap<ExecutionId, Arc<RunControl>>>,
    pub shutdown: CancellationToken,
    pub workers: Mutex<Vec<JoinHandle<()>>>,
    pub owner: String,
}

/// Collects the parts of an [`Engine`]. Only the repository and the model are required.
pub struct EngineBuilder {
    repository: Arc<dyn ExecutionRepository>,
    model: Arc<dyn ModelGateway>,
    tools: Option<Arc<dyn ToolRegistry>>,
    policy: Option<Arc<dyn PolicyEngine>>,
    context: Option<Arc<dyn ContextBuilder>>,
    scheduler: Option<Arc<dyn ExecutionScheduler>>,
    clock: Option<Arc<dyn Clock>>,
    publishers: Vec<Arc<dyn EventPublisher>>,
    validators: Vec<Arc<dyn ResultValidator>>,
    config: EngineConfig,
}

impl EngineBuilder {
    /// A builder with the defaults: no tools, the default policy and context builder, a local scheduler.
    pub fn new(repository: Arc<dyn ExecutionRepository>, model: Arc<dyn ModelGateway>) -> Self {
        Self {
            repository,
            model,
            tools: None,
            policy: None,
            context: None,
            scheduler: None,
            clock: None,
            publishers: Vec::new(),
            validators: Vec::new(),
            config: EngineConfig::default(),
        }
    }

    /// Where tools are found.
    pub fn tools(mut self, tools: Arc<dyn ToolRegistry>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// The policy engine.
    pub fn policy(mut self, policy: Arc<dyn PolicyEngine>) -> Self {
        self.policy = Some(policy);
        self
    }

    /// The context builder.
    pub fn context(mut self, context: Arc<dyn ContextBuilder>) -> Self {
        self.context = Some(context);
        self
    }

    /// The scheduler.
    pub fn scheduler(mut self, scheduler: Arc<dyn ExecutionScheduler>) -> Self {
        self.scheduler = Some(scheduler);
        self
    }

    /// The clock.
    pub fn clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = Some(clock);
        self
    }

    /// An extra place to publish events, besides the built-in bus.
    pub fn publisher(mut self, publisher: Arc<dyn EventPublisher>) -> Self {
        self.publishers.push(publisher);
        self
    }

    /// An extra check on final answers, besides the built-in ones.
    pub fn validator(mut self, validator: Arc<dyn ResultValidator>) -> Self {
        self.validators.push(validator);
        self
    }

    /// The settings.
    pub fn config(mut self, config: EngineConfig) -> Self {
        self.config = config;
        self
    }

    /// Build the engine and start its workers. Must be called inside a Tokio runtime.
    pub fn build(self) -> Engine {
        let mut validators: Vec<Arc<dyn ResultValidator>> =
            vec![Arc::new(NonEmptyValidator), Arc::new(OutputSchemaValidator)];
        validators.extend(self.validators);
        let inner = Arc::new(Inner {
            repository: self.repository,
            model: self.model,
            tools: self.tools.unwrap_or_else(|| Arc::new(InMemoryToolRegistry::new())),
            policy: self.policy.unwrap_or_else(|| Arc::new(DefaultPolicy::default())),
            context: self.context.unwrap_or_else(|| Arc::new(DefaultContextBuilder::new())),
            scheduler: self.scheduler.unwrap_or_else(|| Arc::new(LocalScheduler::new())),
            clock: self.clock.unwrap_or_else(|| Arc::new(SystemClock)),
            bus: EventBus::new(),
            publishers: self.publishers,
            validators,
            owner: format!("engine-{}", ExecutionId::new()),
            config: self.config,
            runs: Mutex::new(HashMap::new()),
            shutdown: CancellationToken::new(),
            workers: Mutex::new(Vec::new()),
        });
        let engine = Engine { inner };
        let workers = engine.inner.config.workers.max(1);
        let handles: Vec<JoinHandle<()>> = (0..workers)
            .map(|_| {
                let engine = engine.clone();
                tokio::spawn(async move { engine.worker_loop().await })
            })
            .collect();
        *engine.inner.workers.lock() = handles;
        engine
    }
}

/// Changes to records that travel with an execution update.
#[derive(Default)]
pub(crate) struct Change {
    pub events: Vec<NewEvent>,
    pub tool_calls: Vec<ToolCallRecord>,
    pub approvals: Vec<Approval>,
}

impl Change {
    pub fn events(events: Vec<NewEvent>) -> Self {
        Self { events, ..Self::default() }
    }
}

/// The execution engine. Cheap to clone: clones share the same workers and state.
#[derive(Clone)]
pub struct Engine {
    pub(crate) inner: Arc<Inner>,
}

impl Engine {
    /// Start building an engine.
    pub fn builder(repository: Arc<dyn ExecutionRepository>, model: Arc<dyn ModelGateway>) -> EngineBuilder {
        EngineBuilder::new(repository, model)
    }

    /// Create an execution and ask for it to run.
    pub async fn submit(&self, new: NewExecution) -> Result<Execution> {
        let tenant = new.tenant.clone().unwrap_or_else(TenantId::local);
        let created = self.create(new).await?;
        self.start(&tenant, created.id).await
    }

    /// A readable summary of one execution: model, tools, step timings, usage and outcome.
    pub async fn summary(&self, tenant: &TenantId, id: ExecutionId) -> Result<crate::summary::ExecutionSummary> {
        let detail = self.detail(tenant, id).await?;
        let events = self.events(tenant, id, 0).await?;
        Ok(crate::summary::summarize(&detail, &events))
    }

    /// The event bus, for subscribers that want every execution.
    pub fn bus(&self) -> &EventBus {
        &self.inner.bus
    }

    /// The tool registry.
    pub fn tools(&self) -> &Arc<dyn ToolRegistry> {
        &self.inner.tools
    }

    /// The current time, from the engine's clock.
    pub fn now(&self) -> Timestamp {
        self.inner.clock.now()
    }

    /// Stop gracefully: workers stop at the next step, leaving executions as they are so that
    /// [`recover`](ExecutionEngine::recover) can continue them later.
    pub async fn shutdown(&self) {
        self.inner.shutdown.cancel();
        self.inner.scheduler.close();
        let handles: Vec<JoinHandle<()>> = std::mem::take(&mut *self.inner.workers.lock());
        for handle in handles {
            let _ = handle.await;
        }
    }

    /// Stop everything at once, as a crash would: no cleanup, no final writes. For tests of recovery.
    pub fn abort(&self) {
        self.inner.shutdown.cancel();
        self.inner.scheduler.close();
        for handle in self.inner.workers.lock().iter() {
            handle.abort();
        }
    }

    pub(crate) fn event(&self, kind: EventKind, payload: Value) -> NewEvent {
        NewEvent::new(kind, self.now(), redact(payload))
    }

    pub(crate) fn publish(&self, events: &[ExecutionEvent]) {
        self.inner.bus.publish(events);
        for publisher in &self.inner.publishers {
            publisher.publish(events);
        }
    }

    pub(crate) async fn commit(&self, commit: Commit) -> Result<zorix_core::Committed> {
        let done = self.inner.repository.commit(commit).await?;
        self.publish(&done.events);
        Ok(done)
    }

    async fn scoped(&self, tenant: &TenantId, id: ExecutionId) -> Result<Execution> {
        self.inner.repository.get(tenant, id).await?.ok_or(Error::NotFound(id))
    }

    /// Read the execution, let `change` modify it, and commit. If somebody else committed in between, read again and
    /// repeat, so a change is never applied to stale data.
    pub(crate) async fn update<F>(&self, id: ExecutionId, mut change: F) -> Result<Execution>
    where
        F: FnMut(&mut Execution, Timestamp) -> Result<Change>,
    {
        for _ in 0..8 {
            let current = self.inner.repository.load(id).await?.ok_or(Error::NotFound(id))?;
            let mut next = current.clone();
            let change = change(&mut next, self.now())?;
            let mut commit = Commit::new(next, current.version);
            commit.events = change.events;
            commit.tool_calls = change.tool_calls;
            commit.approvals = change.approvals;
            match self.commit(commit).await {
                Ok(done) => return Ok(done.execution),
                Err(Error::Conflict(_)) => continue,
                Err(other) => return Err(other),
            }
        }
        Err(Error::Conflict(id))
    }

    async fn worker_loop(self) {
        while let Some(id) = self.inner.scheduler.next().await {
            if std::panic::AssertUnwindSafe(self.drive(id)).catch_unwind().await.is_err() {
                tracing::error!(execution = %id, "a worker panicked; recovery will pick the execution up again");
            }
        }
    }

    /// Wait until the execution is not going anywhere by itself (finished, waiting for a person, paused or not started).
    async fn wait_settled(&self, tenant: &TenantId, id: ExecutionId, limit: Option<Duration>) -> Result<Execution> {
        let mut wake = self.inner.bus.subscribe();
        let deadline = limit.map(|d| tokio::time::Instant::now() + d);
        loop {
            let current = self.scoped(tenant, id).await?;
            let settled = current.state.is_terminal()
                || matches!(
                    current.state,
                    ExecutionState::Created | ExecutionState::WaitingForApproval | ExecutionState::Paused
                );
            if settled || deadline.is_some_and(|d| tokio::time::Instant::now() >= d) {
                return Ok(current);
            }
            tokio::select! {
                _ = wake.recv() => {}
                _ = tokio::time::sleep(Duration::from_millis(250)) => {}
            }
        }
    }
}

#[async_trait]
impl ExecutionEngine for Engine {
    async fn create(&self, new: NewExecution) -> Result<Execution> {
        new.validate()?;
        let execution = Execution::new(ExecutionId::new(), new, self.now());
        let created = self.event(
            EventKind::Created,
            json!({
                "model": execution.request.model,
                "prompt_chars": execution.request.prompt.chars().count(),
                "conversation": execution.conversation,
                "parent": execution.parent,
                "grants": execution.grants.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
            }),
        );
        match self.inner.repository.create(execution, vec![created]).await? {
            CreateOutcome::Created(done) => {
                self.publish(&done.events);
                Ok(done.execution)
            }
            CreateOutcome::Existing(existing) => Ok(existing),
        }
    }

    async fn start(&self, tenant: &TenantId, id: ExecutionId) -> Result<Execution> {
        let current = self.scoped(tenant, id).await?;
        match current.state {
            ExecutionState::Created => {}
            ExecutionState::Queued
            | ExecutionState::Running
            | ExecutionState::WaitingForModel
            | ExecutionState::WaitingForTool
            | ExecutionState::WaitingForApproval => return Ok(current),
            from => return Err(InvalidTransition { from, trigger: Trigger::Enqueue }.into()),
        }
        let started = self
            .update(id, |execution, now| {
                execution.apply(Trigger::Enqueue, now)?;
                Ok(Change::events(vec![self.event(EventKind::Queued, json!({}))]))
            })
            .await?;
        self.inner.scheduler.enqueue(id).await?;
        Ok(started)
    }

    async fn get(&self, tenant: &TenantId, id: ExecutionId) -> Result<Execution> {
        self.scoped(tenant, id).await
    }

    async fn detail(&self, tenant: &TenantId, id: ExecutionId) -> Result<ExecutionDetail> {
        let execution = self.scoped(tenant, id).await?;
        Ok(ExecutionDetail {
            tool_calls: self.inner.repository.tool_calls(id).await?,
            approvals: self.inner.repository.approvals(id).await?,
            execution,
        })
    }

    async fn list(&self, tenant: &TenantId, filter: &ListFilter) -> Result<Vec<Execution>> {
        self.inner.repository.list(tenant, filter).await
    }

    async fn events(&self, tenant: &TenantId, id: ExecutionId, after_seq: u64) -> Result<Vec<ExecutionEvent>> {
        self.scoped(tenant, id).await?;
        self.inner.repository.events(id, after_seq, 100_000).await
    }

    async fn subscribe(
        &self,
        tenant: &TenantId,
        id: ExecutionId,
        after_seq: u64,
    ) -> Result<BoxStream<'static, ExecutionEvent>> {
        self.scoped(tenant, id).await?;
        Ok(follow(self.inner.repository.clone(), &self.inner.bus, id, after_seq))
    }

    async fn settle(&self, tenant: &TenantId, id: ExecutionId) -> Result<Execution> {
        self.wait_settled(tenant, id, None).await
    }

    async fn pause(&self, tenant: &TenantId, id: ExecutionId) -> Result<Execution> {
        let current = self.scoped(tenant, id).await?;
        let active = self.inner.runs.lock().get(&id).cloned();
        if let Some(control) = active {
            control.pause.store(true, std::sync::atomic::Ordering::SeqCst);
            return Ok(current);
        }
        self.update(id, |execution, now| {
            execution.apply(Trigger::Pause, now)?;
            Ok(Change::events(vec![self.event(EventKind::Paused, json!({}))]))
        })
        .await
    }

    async fn resume(&self, tenant: &TenantId, id: ExecutionId) -> Result<Execution> {
        self.scoped(tenant, id).await?;
        let resumed = self
            .update(id, |execution, now| {
                execution.apply(Trigger::Resume, now)?;
                Ok(Change::events(vec![
                    self.event(EventKind::Resumed, json!({})),
                    self.event(EventKind::Queued, json!({})),
                ]))
            })
            .await?;
        self.inner.scheduler.enqueue(id).await?;
        Ok(resumed)
    }

    async fn cancel(&self, tenant: &TenantId, id: ExecutionId, reason: Option<String>) -> Result<Execution> {
        let current = self.scoped(tenant, id).await?;
        if current.state.is_terminal() {
            return Err(InvalidTransition { from: current.state, trigger: Trigger::Cancel }.into());
        }
        let active = self.inner.runs.lock().get(&id).cloned();
        if let Some(control) = active {
            *control.reason.lock() = reason;
            control.cancel.cancel();
            return self.wait_terminal(tenant, id).await;
        }
        let pending: Vec<Approval> = self
            .inner
            .repository
            .approvals(id)
            .await?
            .into_iter()
            .filter(|a| a.status == ApprovalStatus::Pending)
            .collect();
        self.update(id, |execution, now| {
            execution.apply(Trigger::Cancel, now)?;
            execution.cancel_reason = reason.clone();
            let approvals = pending
                .iter()
                .cloned()
                .map(|mut approval| {
                    approval.status = ApprovalStatus::Denied;
                    approval.decided_at = Some(now);
                    approval.decided_by = Some("system".into());
                    approval.note = Some("The execution was cancelled.".into());
                    approval
                })
                .collect();
            Ok(Change {
                events: vec![self.event(EventKind::Cancelled, json!({ "reason": reason }))],
                approvals,
                ..Change::default()
            })
        })
        .await
    }

    async fn decide(
        &self,
        tenant: &TenantId,
        id: ExecutionId,
        approval: ApprovalId,
        decision: ApprovalDecision,
    ) -> Result<Execution> {
        let current = self.scoped(tenant, id).await?;
        if current.state != ExecutionState::WaitingForApproval {
            return Err(Error::InvalidInput("This execution is not waiting for an approval.".into()));
        }
        if decision.decided_by.trim().is_empty() || decision.decided_by.chars().count() > 128 {
            return Err(Error::InvalidInput("Say who is deciding (1 to 128 characters).".into()));
        }
        let pending = self
            .inner
            .repository
            .approvals(id)
            .await?
            .into_iter()
            .find(|a| a.id == approval)
            .ok_or_else(|| Error::Missing("This execution has no such approval.".into()))?;
        if pending.status != ApprovalStatus::Pending {
            return Err(Error::InvalidInput("This approval was already answered.".into()));
        }
        if decision.call_hash.as_ref().is_some_and(|hash| *hash != pending.call_hash) {
            return Err(Error::InvalidInput("The answer was given for a different call than the one waiting.".into()));
        }
        let decided = self
            .update(id, |execution, now| {
                execution.apply(Trigger::ApprovalResolved, now)?;
                let mut answered = pending.clone();
                answered.status = if decision.approve { ApprovalStatus::Approved } else { ApprovalStatus::Denied };
                answered.decided_at = Some(now);
                answered.decided_by = Some(decision.decided_by.clone());
                answered.note = decision.note.clone();
                let kind = if decision.approve { EventKind::ApprovalGranted } else { EventKind::ApprovalDenied };
                let event = self.event(
                    kind,
                    json!({
                        "approval_id": answered.id,
                        "call_id": answered.call_id,
                        "tool": answered.tool,
                        "decided_by": answered.decided_by,
                        "note": answered.note,
                    }),
                );
                Ok(Change {
                    events: vec![event, self.event(EventKind::Queued, json!({}))],
                    approvals: vec![answered],
                    ..Change::default()
                })
            })
            .await?;
        self.inner.scheduler.enqueue(id).await?;
        Ok(decided)
    }

    async fn recover(&self) -> Result<RecoveryReport> {
        use ExecutionState as S;
        let mut report = RecoveryReport::default();
        if self.inner.config.sole_owner {
            self.inner.repository.release_all_leases().await?;
        }
        let states = [S::Queued, S::Running, S::WaitingForModel, S::WaitingForTool, S::WaitingForApproval, S::Paused];
        for execution in self.inner.repository.in_states(&states).await? {
            let id = execution.id;
            if self.inner.runs.lock().contains_key(&id) {
                report.in_use += 1;
                continue;
            }
            match execution.state {
                S::Queued => {
                    self.inner.scheduler.enqueue(id).await?;
                    report.untouched += 1;
                }
                S::WaitingForApproval | S::Paused => report.untouched += 1,
                S::Running | S::WaitingForModel | S::WaitingForTool => {
                    // A live engine that works on it holds its lease. Only take over what nobody holds.
                    let repository = &self.inner.repository;
                    let (owner, ttl) = (&self.inner.owner, self.inner.config.lease_ttl_ms);
                    if !repository.acquire_lease(id, owner, self.now(), ttl).await? {
                        report.in_use += 1;
                        continue;
                    }
                    let outcome = self.take_over(execution.state, id).await;
                    repository.release_lease(id, owner).await?;
                    match outcome? {
                        TakeOver::Requeued => report.requeued.push(id),
                        TakeOver::NeedsDecision => report.needs_decision.push(id),
                    }
                }
                _ => {}
            }
        }
        Ok(report)
    }
}

enum TakeOver {
    Requeued,
    NeedsDecision,
}

impl Engine {
    async fn wait_terminal(&self, tenant: &TenantId, id: ExecutionId) -> Result<Execution> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            let current = self.wait_settled(tenant, id, Some(Duration::from_secs(1))).await?;
            if current.state.is_terminal() || tokio::time::Instant::now() >= deadline {
                return Ok(current);
            }
        }
    }

    /// Continue an execution that a dead worker left in the middle of a step.
    async fn take_over(&self, state: ExecutionState, id: ExecutionId) -> Result<TakeOver> {
        if state == ExecutionState::WaitingForTool {
            let records = self.inner.repository.tool_calls(id).await?;
            if let Some(record) = records.into_iter().find(|r| r.status == ToolCallStatus::Started && !r.idempotent) {
                self.ask_about_unknown_outcome(id, record).await?;
                return Ok(TakeOver::NeedsDecision);
            }
        }
        self.requeue(id).await?;
        Ok(TakeOver::Requeued)
    }

    async fn requeue(&self, id: ExecutionId) -> Result<()> {
        self.update(id, |execution, now| {
            execution.apply(Trigger::Recover, now)?;
            Ok(Change::events(vec![
                self.event(EventKind::Recovered, json!({})),
                self.event(EventKind::Queued, json!({})),
            ]))
        })
        .await?;
        self.inner.scheduler.enqueue(id).await
    }

    /// A non-repeatable tool call was running when the process stopped. A person decides whether to run it again.
    async fn ask_about_unknown_outcome(&self, id: ExecutionId, mut record: ToolCallRecord) -> Result<()> {
        let approval = Approval {
            id: ApprovalId::new(),
            execution_id: id,
            call_id: record.call_id.clone(),
            tool: record.tool.clone(),
            arguments: record.arguments.clone(),
            call_hash: record.call_hash.clone(),
            reason: format!(
                "ZORIX stopped while {} was running, so it is not known whether it finished. Approve to run it again, or deny to skip it.",
                record.tool
            ),
            status: ApprovalStatus::Pending,
            requested_at: self.now(),
            decided_at: None,
            decided_by: None,
            note: None,
        };
        record.status = ToolCallStatus::AwaitingApproval;
        self.update(id, |execution, now| {
            execution.apply(Trigger::AwaitApproval, now)?;
            let event = self.event(
                EventKind::WaitingApproval,
                json!({
                    "approval_id": approval.id,
                    "call_id": approval.call_id,
                    "tool": approval.tool,
                    "arguments": approval.arguments,
                    "call_hash": approval.call_hash,
                    "reason": approval.reason,
                    "outcome_unknown": true,
                }),
            );
            Ok(Change { events: vec![event], tool_calls: vec![record.clone()], approvals: vec![approval.clone()] })
        })
        .await?;
        Ok(())
    }
}
