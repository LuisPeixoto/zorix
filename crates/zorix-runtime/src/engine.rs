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
    AppSettings, Approval, ApprovalDecision, ApprovalId, ApprovalStatus, ChildResult, Clock, Commit, ContextBuilder,
    ContextSettings, CreateOutcome, DelegateRequest, DelegateTask, DelegationPlanner, Error, EventKind, EventPublisher,
    Execution, ExecutionDetail, ExecutionEngine, ExecutionEvent, ExecutionId, ExecutionRepository, ExecutionScheduler,
    ExecutionState, InvalidTransition, ListFilter, ModelGateway, NewEvent, NewExecution, NoCalibration, PolicyEngine,
    RecoveryReport, Result, ResultValidator, SettingsCell, SystemClock, TenantId, Timestamp, TokenCalibration,
    ToolCallRecord, ToolCallStatus, ToolRegistry, Trigger, Usage,
};

use crate::context::DefaultContextBuilder;
use crate::events::{EventBus, follow};
use crate::policy::DefaultPolicy;
use crate::retry::RetryPolicy;
use crate::scheduler::LocalScheduler;
use crate::tools::InMemoryToolRegistry;
use crate::validators::{NonEmptyValidator, OutputSchemaValidator};
use zorix_core::redact;

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
    /// The folder file tools are confined to when an execution does not name its own. `None` means no workspace.
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
    /// Set when the engine lost the execution's lease: the run stops without writing, and the engine that holds it carries on.
    pub lost: AtomicBool,
}

pub(crate) struct Inner {
    pub repository: Arc<dyn ExecutionRepository>,
    pub model: Arc<dyn ModelGateway>,
    pub tools: Arc<dyn ToolRegistry>,
    pub policy: Arc<dyn PolicyEngine>,
    pub context: Arc<dyn ContextBuilder>,
    pub calibration: Arc<dyn TokenCalibration>,
    pub settings: SettingsCell,
    pub scheduler: Arc<dyn ExecutionScheduler>,
    pub clock: Arc<dyn Clock>,
    pub bus: EventBus,
    pub live: crate::live::LiveBus,
    pub publishers: Vec<Arc<dyn EventPublisher>>,
    pub validators: Vec<Arc<dyn ResultValidator>>,
    pub planner: Option<Arc<dyn DelegationPlanner>>,
    pub config: EngineConfig,
    pub runs: Mutex<HashMap<ExecutionId, Arc<RunControl>>>,
    /// The executions a worker of this engine is driving right now: a second worker that picks one up waits for the first to finish.
    pub driving: Mutex<std::collections::HashSet<ExecutionId>>,
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
    calibration: Option<Arc<dyn TokenCalibration>>,
    scheduler: Option<Arc<dyn ExecutionScheduler>>,
    clock: Option<Arc<dyn Clock>>,
    publishers: Vec<Arc<dyn EventPublisher>>,
    validators: Vec<Arc<dyn ResultValidator>>,
    planner: Option<Arc<dyn DelegationPlanner>>,
    config: EngineConfig,
    settings: Option<SettingsCell>,
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
            calibration: None,
            settings: None,
            scheduler: None,
            clock: None,
            publishers: Vec::new(),
            validators: Vec::new(),
            planner: None,
            config: EngineConfig::default(),
        }
    }

    /// Where tools are found.
    pub fn tools(mut self, tools: Arc<dyn ToolRegistry>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Who turns the tasks of a delegate call into executions. Without one, `agents.delegate` fails with a sentence.
    pub fn delegation(mut self, planner: Arc<dyn DelegationPlanner>) -> Self {
        self.planner = Some(planner);
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

    /// The settings the engine reads while it runs (whether long conversations are summarised). A host passes the cell it also saves
    /// changes to, so they take effect at once. An engine built without one keeps the old behaviour: it leaves old turns out and does not
    /// summarise them.
    pub fn settings(mut self, settings: SettingsCell) -> Self {
        self.settings = Some(settings);
        self
    }

    /// Where token estimates are corrected per model. Without one, estimates are used as they are.
    pub fn calibration(mut self, calibration: Arc<dyn TokenCalibration>) -> Self {
        self.calibration = Some(calibration);
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
            calibration: self.calibration.unwrap_or_else(|| Arc::new(NoCalibration)),
            settings: self.settings.unwrap_or_else(|| {
                SettingsCell::new(AppSettings {
                    context: ContextSettings { summaries: false },
                    ..AppSettings::default()
                })
            }),
            scheduler: self.scheduler.unwrap_or_else(|| Arc::new(LocalScheduler::new())),
            clock: self.clock.unwrap_or_else(|| Arc::new(SystemClock)),
            bus: EventBus::new(),
            live: crate::live::LiveBus::new(),
            publishers: self.publishers,
            planner: self.planner,
            validators,
            owner: format!("engine-{}", ExecutionId::new()),
            config: self.config,
            runs: Mutex::new(HashMap::new()),
            driving: Mutex::new(std::collections::HashSet::new()),
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
        self.wake_parent_of(&done.execution);
        Ok(done)
    }

    /// A child that has finished may be the last one its parent waits for. This runs in the background so the child's own commit
    /// is never slowed or failed by it; `recover` finds anything this misses.
    fn wake_parent_of(&self, execution: &Execution) {
        let Some(parent) = execution.parent else { return };
        if !execution.state.is_terminal() {
            return;
        }
        let engine = self.clone();
        tokio::spawn(async move {
            if let Err(error) = engine.reconcile_children(parent).await {
                tracing::warn!(%parent, %error, "could not settle the children of an execution");
            }
        });
    }

    /// If `parent` waits for children and every one of them has finished, give the parent their results, add what they used to its
    /// own usage, and queue it. Returns whether it did. Safe to call at any time and from several places at once.
    pub(crate) async fn reconcile_children(&self, parent_id: ExecutionId) -> Result<bool> {
        let repository = &self.inner.repository;
        let Some(parent) = repository.load(parent_id).await? else { return Ok(false) };
        if parent.state != ExecutionState::WaitingForChildren {
            return Ok(false);
        }
        let Some(record) =
            repository.tool_calls(parent_id).await?.into_iter().find(|r| r.status == ToolCallStatus::Delegated)
        else {
            return Ok(false);
        };
        let ids: Vec<ExecutionId> = record
            .result
            .as_ref()
            .and_then(|r| r["delegated"].as_array())
            .map(|list| list.iter().filter_map(|d| d["child"].as_str().and_then(|s| s.parse().ok())).collect())
            .unwrap_or_default();
        let mut children = Vec::with_capacity(ids.len());
        for id in ids {
            match repository.get(&parent.tenant, id).await? {
                Some(child) if child.state.is_terminal() => children.push(child),
                _ => return Ok(false),
            }
        }
        let tasks = DelegateRequest::parse(&record.arguments).map(|r| r.tasks).unwrap_or_default();
        let results: Vec<ChildResult> = children
            .iter()
            .enumerate()
            .map(|(i, child)| {
                let task = tasks.get(i).cloned().unwrap_or_else(|| DelegateTask {
                    agent: child.agent.as_ref().map(|a| a.slug.to_string()).unwrap_or_default(),
                    task: child.request.prompt.clone(),
                    input: None,
                    expected_output: None,
                    context: Vec::new(),
                    files: Vec::new(),
                });
                ChildResult::of(&task, child)
            })
            .collect();
        let mut rolled = Usage::default();
        for child in &children {
            rolled.add(&child.usage);
        }
        const ALREADY: &str = "the children were already settled";
        let settled = self
            .update(parent_id, |execution, now| {
                if execution.state != ExecutionState::WaitingForChildren {
                    return Err(Error::InvalidInput(ALREADY.into()));
                }
                execution.apply(Trigger::ChildrenSettled, now)?;
                // What the children used is what the parent used: the next limit check sees it.
                execution.usage.add(&rolled);
                execution.tool_calls_requested = execution.tool_calls_requested.saturating_add(rolled.tool_calls);
                let mut done = record.clone();
                done.status = ToolCallStatus::Succeeded;
                done.result = Some(json!({ "results": results }));
                done.finished_at = Some(now);
                let mut events: Vec<NewEvent> = results
                    .iter()
                    .map(|r| {
                        self.event(
                            EventKind::ChildSettled,
                            json!({
                                "child": r.child_id, "agent": r.agent, "state": r.state,
                                "steps": r.steps, "tool_calls": r.tool_calls, "cost_micros": r.cost_micros,
                            }),
                        )
                    })
                    .collect();
                events.push(self.event(EventKind::Queued, json!({})));
                Ok(Change { events, tool_calls: vec![done], ..Change::default() })
            })
            .await;
        match settled {
            Ok(_) => {
                self.inner.scheduler.enqueue(parent_id).await?;
                Ok(true)
            }
            Err(Error::InvalidInput(message)) if message == ALREADY => Ok(false),
            Err(other) => Err(other),
        }
    }

    /// Whether anything below `id` waits for a person.
    async fn tree_waits_for_a_person(&self, tenant: &TenantId, id: ExecutionId) -> Result<bool> {
        let below = self.inner.repository.descendants(tenant, id).await?;
        Ok(below.iter().any(|e| e.state == ExecutionState::WaitingForApproval))
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
                )
                // Waiting for children is still working, unless something below waits for a person: then the tree is waiting.
                || (current.state == ExecutionState::WaitingForChildren
                    && self.tree_waits_for_a_person(tenant, id).await.unwrap_or(false));
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
            | ExecutionState::WaitingForApproval
            | ExecutionState::WaitingForChildren => return Ok(current),
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

    async fn live(&self, tenant: &TenantId, id: ExecutionId) -> Result<BoxStream<'static, zorix_core::LiveEvent>> {
        self.scoped(tenant, id).await?;
        Ok(self.inner.live.watch(id))
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
        let top = self.cancel_one(tenant, id, reason.clone()).await?;
        // Cancelling a run cancels everything it started, top down, so no parent is woken only to find its children gone.
        let below = self.inner.repository.descendants(tenant, id).await?;
        let because = Some(reason.unwrap_or_else(|| "The execution that asked for this was cancelled.".into()));
        for child in below.into_iter().filter(|e| !e.state.is_terminal()) {
            if let Err(error) = self.cancel_one(tenant, child.id, because.clone()).await {
                tracing::warn!(execution = %child.id, %error, "a sub-run could not be cancelled");
            }
        }
        Ok(top)
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
        // An answer names the exact call it is for. Without the hash, "yes" could be applied to a call the person never saw.
        match decision.call_hash.as_ref() {
            None => {
                return Err(Error::InvalidInput(
                    "An answer must name the call it is for (its call hash), so it cannot be applied to another one."
                        .into(),
                ));
            }
            Some(hash) if *hash != pending.call_hash => {
                return Err(Error::InvalidInput(
                    "The answer was given for a different call than the one waiting.".into(),
                ));
            }
            Some(_) => {}
        }
        if decision.approve
            && decision.automatic
            && self.inner.tools.get(&pending.tool).is_some_and(|tool| tool.spec().approval_by_person_only)
        {
            return Err(Error::InvalidInput(format!(
                "{} must be approved by a person who has seen it; an automatic mode cannot approve it.",
                pending.tool
            )));
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
        let states = [
            S::Queued,
            S::Running,
            S::WaitingForModel,
            S::WaitingForTool,
            S::WaitingForApproval,
            S::WaitingForChildren,
            S::Paused,
        ];
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
                S::WaitingForChildren => {
                    // Children that finished while this program was down (or between the child's commit and the wake-up) are
                    // given to their parent now; children that did not finish recover on their own.
                    if self.reconcile_children(id).await? {
                        report.requeued.push(id);
                    } else {
                        report.untouched += 1;
                    }
                }
                S::Running | S::WaitingForModel | S::WaitingForTool => {
                    // A live engine that works on it holds its lease. Only take over what nobody holds.
                    let repository = &self.inner.repository;
                    let (owner, ttl) = (&self.inner.owner, self.inner.config.lease_ttl_ms);
                    if !repository.acquire_lease(id, owner, self.now(), ttl).await? {
                        report.in_use += 1;
                        continue;
                    }
                    let outcome = self.take_over(execution.state, id).await;
                    // Let go before the run is queued: the worker that picks it up holds this same lease (the owner is the engine), and
                    // releasing after it started would delete the lease under it.
                    repository.release_lease(id, owner).await?;
                    match outcome? {
                        TakeOver::Requeued => {
                            self.inner.scheduler.enqueue(id).await?;
                            report.requeued.push(id);
                        }
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
    /// Cancel one execution, not what it started.
    async fn cancel_one(&self, tenant: &TenantId, id: ExecutionId, reason: Option<String>) -> Result<Execution> {
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
        self.mark_recovered(id).await?;
        Ok(TakeOver::Requeued)
    }

    /// Move the execution back to `Queued` with the events that say so. The caller queues it for a worker once it has let go of the lease.
    async fn mark_recovered(&self, id: ExecutionId) -> Result<()> {
        self.update(id, |execution, now| {
            execution.apply(Trigger::Recover, now)?;
            Ok(Change::events(vec![
                self.event(EventKind::Recovered, json!({})),
                self.event(EventKind::Queued, json!({})),
            ]))
        })
        .await?;
        Ok(())
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
