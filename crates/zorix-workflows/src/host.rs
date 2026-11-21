//! The host: everything that starts workflows by itself (ADR-0034).
//!
//! One task wakes every second and does four things: it fires the schedules that are due (and records the ones that were missed
//! while nothing was running), starts workflows for events that happened, advances the runs that are going, and keeps the
//! triggers in step with what is published. A webhook listener, when switched on, is a second task. Both stop together.

use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use zorix_core::{
    AgentSlug, Clock, Error, EventOn, Result, RunState, Secret, StepKind, TenantId, Timestamp, TriggerState, Workflow,
    WorkflowId, WorkflowRun, WorkflowSpec,
};
use zorix_providers::SecretStore;

use crate::engine::{StartRequest, WorkflowEngine};
use crate::schedule::{fires_between, next_fire};

/// A schedule that is later than this when the host looks at it was missed, not late: it is recorded and not run.
pub const MISS_GRACE_MS: i64 = 120_000;
/// Most missed fires counted for one trigger (a long absence stays cheap).
const MISSED_CAP: usize = 1_000;
/// How long the triggers are trusted before they are read again.
const RECONCILE_MS: i64 = 10_000;

const CURSOR_RUNS: &str = "cursor.workflow_runs";
const CURSOR_EXECUTIONS: &str = "cursor.executions";

/// What the host does.
#[derive(Clone, Debug)]
pub struct HostConfig {
    /// Where to listen for webhooks; none means nothing listens.
    pub webhook: Option<SocketAddr>,
    /// How often the host wakes.
    pub tick: Duration,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self { webhook: None, tick: Duration::from_secs(1) }
    }
}

/// An agent or team run that ended, as the host needs to know it.
#[derive(Clone, Debug, PartialEq)]
pub struct FinishedExecution {
    /// The execution.
    pub id: String,
    /// The agent, when an agent made it.
    pub agent: Option<AgentSlug>,
    /// Whether it ended well.
    pub ok: bool,
    /// When it ended.
    pub finished_at: Timestamp,
    /// The answer, when it ended well.
    pub text: String,
    /// Why, when it did not.
    pub error: String,
    /// Whether a workflow started it (those do not start workflows, so a workflow cannot start itself through an agent).
    pub from_workflow: bool,
}

/// Where the host learns that agent and team runs ended.
#[async_trait]
pub trait ExecutionFeed: Send + Sync {
    /// The top-level runs that ended after `since`, oldest first.
    async fn finished_since(&self, since: Timestamp) -> Result<Vec<FinishedExecution>>;
}

/// The feed over the execution engine.
pub struct EngineFeed {
    engine: Arc<dyn zorix_core::ExecutionEngine>,
    tenant: TenantId,
}

impl EngineFeed {
    /// A feed of one tenant's executions.
    pub fn new(engine: Arc<dyn zorix_core::ExecutionEngine>, tenant: TenantId) -> Self {
        Self { engine, tenant }
    }
}

#[async_trait]
impl ExecutionFeed for EngineFeed {
    async fn finished_since(&self, since: Timestamp) -> Result<Vec<FinishedExecution>> {
        use zorix_core::{ExecutionState, ListFilter};
        let filter = ListFilter {
            states: vec![ExecutionState::Completed, ExecutionState::Failed],
            roots_only: true,
            limit: 100,
            ..ListFilter::default()
        };
        let mut found: Vec<FinishedExecution> = self
            .engine
            .list(&self.tenant, &filter)
            .await?
            .into_iter()
            .filter_map(|e| {
                let finished = e.finished_at?;
                (finished > since).then(|| FinishedExecution {
                    id: e.id.to_string(),
                    agent: e.agent.as_ref().map(|a| a.slug.clone()),
                    ok: e.state == ExecutionState::Completed,
                    finished_at: finished,
                    text: e.result.map(|r| r.text).unwrap_or_default(),
                    error: e.failure.map(|f| f.message).unwrap_or_default(),
                    from_workflow: e.request.metadata.contains_key("workflow_run"),
                })
            })
            .collect();
        found.sort_by_key(|f| f.finished_at);
        Ok(found)
    }
}

pub(crate) struct Inner {
    pub(crate) engine: Arc<WorkflowEngine>,
    feed: Arc<dyn ExecutionFeed>,
    secrets: Arc<dyn SecretStore>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) tenant: TenantId,
    refresh: AtomicBool,
    live: Mutex<Live>,
}

#[derive(Default)]
struct Live {
    /// The enabled workflows with the definition their next trigger uses.
    workflows: Vec<(Workflow, WorkflowSpec)>,
    read_at: Option<Timestamp>,
}

/// Starts workflows by itself.
pub struct WorkflowHost {
    inner: Arc<Inner>,
    cancel: CancellationToken,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    webhook: Mutex<Option<SocketAddr>>,
}

fn storage(error: impl std::fmt::Display) -> Error {
    Error::Storage(error.to_string())
}

pub(crate) fn token_key(id: WorkflowId) -> String {
    format!("workflow-webhook.{id}")
}

pub(crate) fn signing_key(id: WorkflowId) -> String {
    format!("workflow-signing.{id}")
}

/// A random secret as hex, from the operating system's generator.
pub fn random_secret() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| Error::Internal(format!("no source of randomness: {e}")))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

impl Inner {
    pub(crate) async fn secret(&self, key: String) -> Result<Option<String>> {
        let secrets = self.secrets.clone();
        tokio::task::spawn_blocking(move || secrets.get(&key))
            .await
            .map_err(storage)?
            .map(|s| s.map(|s| s.expose().to_string()))
            .map_err(storage)
    }

    async fn secret_or_create(&self, key: String) -> Result<String> {
        if let Some(found) = self.secret(key.clone()).await? {
            return Ok(found);
        }
        let fresh = random_secret()?;
        let (secrets, value) = (self.secrets.clone(), fresh.clone());
        tokio::task::spawn_blocking(move || secrets.set(&key, &Secret::new(value)))
            .await
            .map_err(storage)?
            .map_err(storage)?;
        Ok(fresh)
    }

    async fn forget(&self, key: String) -> Result<()> {
        let secrets = self.secrets.clone();
        tokio::task::spawn_blocking(move || secrets.remove(&key)).await.map_err(storage)?.map_err(storage)?;
        Ok(())
    }

    /// The enabled workflows with their live definitions, re-read when stale or when asked.
    pub(crate) async fn workflows(&self, now: Timestamp, force: bool) -> Result<Vec<(Workflow, WorkflowSpec)>> {
        {
            let live = self.live.lock();
            if !force && live.read_at.is_some_and(|at| (now.millis() - at.millis()) < RECONCILE_MS) {
                return Ok(live.workflows.clone());
            }
        }
        let store = self.engine.store();
        let mut found = Vec::new();
        for workflow in store.workflows(&self.tenant).await? {
            if !workflow.enabled {
                continue;
            }
            let Some(version) = workflow.live_version else { continue };
            if let Some(v) = store.version(&self.tenant, workflow.id, version).await? {
                found.push((workflow, v.spec));
            }
        }
        let mut live = self.live.lock();
        live.workflows = found.clone();
        live.read_at = Some(now);
        Ok(found)
    }

    pub(crate) async fn fire(
        &self,
        workflow: &Workflow,
        step: &str,
        payload: Value,
        chain: u32,
        from: Option<&WorkflowRun>,
    ) -> Option<WorkflowRun> {
        let request = StartRequest {
            workflow: workflow.id,
            trigger: Some(step.to_string()),
            payload,
            test: false,
            chain,
            from_run: from.map(|r| r.id),
        };
        match self.engine.start(request).await {
            Ok(run) => Some(run),
            Err(error) => {
                tracing::warn!(workflow = %workflow.slug, step, "a trigger fired but the run did not start: {error}");
                None
            }
        }
    }

    // -------------------------------------------------------------------------------------------------------------
    // Schedules

    async fn reconcile(&self, now: Timestamp, force: bool) -> Result<()> {
        let store = self.engine.store();
        let live = self.workflows(now, force).await?;
        let states = store.trigger_states(&self.tenant).await?;
        let by_key: HashMap<(WorkflowId, String), &TriggerState> =
            states.iter().map(|s| ((s.workflow, s.step.clone()), s)).collect();
        let wanted: BTreeMap<(WorkflowId, String), &StepKind> = live
            .iter()
            .flat_map(|(w, spec)| {
                spec.triggers()
                    .filter(|s| matches!(s.kind, StepKind::Schedule { .. }))
                    .map(move |s| ((w.id, s.id.clone()), &s.kind))
            })
            .collect();
        // Triggers that are gone, or whose workflow is off, lose their state.
        let known: std::collections::HashSet<WorkflowId> = states.iter().map(|s| s.workflow).collect();
        for workflow in known {
            if !wanted.keys().any(|(w, _)| *w == workflow) {
                store.clear_trigger_states(&self.tenant, workflow).await?;
            }
        }
        for ((workflow, step), kind) in &wanted {
            let StepKind::Schedule { when, timezone } = kind else { continue };
            let existing = by_key.get(&(*workflow, step.clone()));
            if existing.is_some_and(|s| s.next_at.is_some()) {
                continue;
            }
            let next = next_fire(when, timezone.as_deref(), now).map_err(Error::InvalidInput)?;
            let state = TriggerState {
                workflow: *workflow,
                step: step.clone(),
                next_at: next,
                last_fired: existing.and_then(|s| s.last_fired),
                missed: existing.map(|s| s.missed).unwrap_or(0),
                last_missed: existing.and_then(|s| s.last_missed),
            };
            store.put_trigger_state(&self.tenant, &state).await?;
        }
        Ok(())
    }

    async fn fire_due(&self, now: Timestamp) -> Result<()> {
        let store = self.engine.store();
        let live = self.workflows(now, false).await?;
        for mut state in store.trigger_states(&self.tenant).await? {
            let Some(due) = state.next_at else { continue };
            if due > now {
                continue;
            }
            let Some((workflow, spec)) = live.iter().find(|(w, _)| w.id == state.workflow) else { continue };
            let Some(step) = spec.step(&state.step) else { continue };
            let StepKind::Schedule { when, timezone } = &step.kind else { continue };
            if now.millis() - due.millis() > MISS_GRACE_MS {
                let missed = fires_between(when, timezone.as_deref(), Timestamp(due.millis() - 1), now, MISSED_CAP)
                    .map_err(Error::InvalidInput)?;
                state.missed += missed.len() as u32;
                state.last_missed = missed.last().copied().or(Some(due));
            } else {
                state.last_fired = Some(now);
                let payload = json!({"scheduled_for": due.to_rfc3339()});
                self.fire(workflow, &state.step, payload, 0, None).await;
            }
            state.next_at = next_fire(when, timezone.as_deref(), now).map_err(Error::InvalidInput)?;
            store.put_trigger_state(&self.tenant, &state).await?;
        }
        Ok(())
    }

    // -------------------------------------------------------------------------------------------------------------
    // Events

    async fn cursor(&self, key: &str, now: Timestamp) -> Result<Timestamp> {
        let store = self.engine.store();
        match store.meta(&self.tenant, key).await? {
            Some(text) => Ok(Timestamp(text.parse().unwrap_or(now.millis()))),
            None => {
                // The first time: what happened before now is history, not news.
                store.set_meta(&self.tenant, key, &now.millis().to_string()).await?;
                Ok(now)
            }
        }
    }

    async fn already_fired(&self, workflow: WorkflowId, key: &str, value: &str) -> Result<bool> {
        let runs = self.engine.store().runs(&self.tenant, Some(workflow), 50).await?;
        Ok(runs.iter().any(|r| r.fired.payload.get(key).and_then(Value::as_str) == Some(value)))
    }

    async fn poll_events(&self, now: Timestamp) -> Result<()> {
        let live = self.workflows(now, false).await?;
        let wants_runs =
            live.iter().any(|(_, s)| s.triggers().any(|t| matches!(t.kind, StepKind::AfterWorkflow { .. })));
        let wants_executions = live.iter().any(|(_, s)| s.triggers().any(|t| matches!(t.kind, StepKind::Event { .. })));
        if wants_runs {
            self.after_workflows(&live, now).await?;
        }
        if wants_executions {
            self.executions(&live, now).await?;
        }
        Ok(())
    }

    async fn after_workflows(&self, live: &[(Workflow, WorkflowSpec)], now: Timestamp) -> Result<()> {
        let store = self.engine.store();
        let since = self.cursor(CURSOR_RUNS, now).await?;
        let mut ended: Vec<WorkflowRun> = store
            .runs(&self.tenant, None, 200)
            .await?
            .into_iter()
            .filter(|r| r.state.is_final() && !r.test && r.finished_at.is_some_and(|f| f >= since))
            .collect();
        ended.sort_by_key(|r| r.finished_at);
        let mut newest = since;
        for run in ended {
            newest = newest.max(run.finished_at.unwrap_or(since));
            for (target, spec) in live {
                if target.id == run.workflow {
                    continue;
                }
                for trigger in spec.triggers() {
                    let StepKind::AfterWorkflow { workflow, on } = &trigger.kind else { continue };
                    if *workflow != run.slug
                        || !matches(*on, run.state == RunState::Completed, run.state == RunState::Failed)
                    {
                        continue;
                    }
                    if self.already_fired(target.id, "run", &run.id.to_string()).await? {
                        continue;
                    }
                    let payload = json!({
                        "run": run.id.to_string(),
                        "workflow": run.slug.to_string(),
                        "state": run.state.as_str(),
                        "error": run.error,
                        "cost_micros": run.cost_micros,
                    });
                    self.fire(target, &trigger.id, payload, run.chain + 1, Some(&run)).await;
                }
            }
        }
        store.set_meta(&self.tenant, CURSOR_RUNS, &newest.millis().to_string()).await
    }

    async fn executions(&self, live: &[(Workflow, WorkflowSpec)], now: Timestamp) -> Result<()> {
        let since = self.cursor(CURSOR_EXECUTIONS, now).await?;
        let mut newest = since;
        for finished in self.feed.finished_since(since).await? {
            newest = newest.max(finished.finished_at);
            if finished.from_workflow {
                continue;
            }
            for (target, spec) in live {
                for trigger in spec.triggers() {
                    let StepKind::Event { on, agent } = &trigger.kind else { continue };
                    if !matches(*on, finished.ok, !finished.ok) || (agent.is_some() && agent != &finished.agent) {
                        continue;
                    }
                    if self.already_fired(target.id, "execution", &finished.id).await? {
                        continue;
                    }
                    let payload = json!({
                        "execution": finished.id,
                        "agent": finished.agent.as_ref().map(|a| a.to_string()),
                        "state": if finished.ok { "completed" } else { "failed" },
                        "text": finished.text,
                        "error": finished.error,
                    });
                    self.fire(target, &trigger.id, payload, 0, None).await;
                }
            }
        }
        self.engine.store().set_meta(&self.tenant, CURSOR_EXECUTIONS, &newest.millis().to_string()).await
    }

    // -------------------------------------------------------------------------------------------------------------
    // One beat

    async fn beat(&self) -> Result<()> {
        let now = self.clock.now();
        let force = self.refresh.swap(false, Ordering::SeqCst);
        self.reconcile(now, force).await?;
        self.fire_due(now).await?;
        self.poll_events(now).await?;
        self.engine.tick().await?;
        Ok(())
    }
}

fn matches(on: EventOn, ok: bool, failed: bool) -> bool {
    match on {
        EventOn::Finished => ok,
        EventOn::Failed => failed,
        EventOn::Any => ok || failed,
    }
}

impl WorkflowHost {
    /// Start the host: recover what a stop left half done, then wake every `config.tick`. Starts the webhook listener when the
    /// configuration names an address.
    pub async fn start(
        engine: Arc<WorkflowEngine>,
        feed: Arc<dyn ExecutionFeed>,
        secrets: Arc<dyn SecretStore>,
        clock: Arc<dyn Clock>,
        config: HostConfig,
    ) -> Result<Self> {
        let tenant = engine.tenant().clone();
        let inner = Arc::new(Inner {
            engine,
            feed,
            secrets,
            clock,
            tenant,
            refresh: AtomicBool::new(false),
            live: Mutex::new(Live::default()),
        });
        inner.engine.recover().await?;
        let cancel = CancellationToken::new();
        let host = Self {
            inner: inner.clone(),
            cancel: cancel.clone(),
            tasks: Mutex::new(Vec::new()),
            webhook: Mutex::new(None),
        };
        // The first round happens before this returns, so what was due while nothing ran is dealt with before anyone looks.
        if let Err(error) = inner.beat().await {
            tracing::warn!("the workflow host could not do its first rounds: {error}");
        }
        let beat = tokio::spawn({
            let inner = inner.clone();
            let cancel = cancel.clone();
            async move {
                loop {
                    tokio::select! {
                        _ = cancel.cancelled() => break,
                        _ = tokio::time::sleep(config.tick) => {}
                    }
                    if let Err(error) = inner.beat().await {
                        tracing::warn!("the workflow host could not do its rounds: {error}");
                    }
                }
            }
        });
        host.tasks.lock().push(beat);
        if let Some(address) = config.webhook {
            let bound = crate::webhook::serve(inner.clone(), address, cancel.clone(), &host.tasks).await?;
            *host.webhook.lock() = Some(bound);
        }
        Ok(host)
    }

    /// Where webhooks arrive, when the listener runs.
    pub fn webhook_addr(&self) -> Option<SocketAddr> {
        *self.webhook.lock()
    }

    /// Read the triggers again at the next beat: a workflow was published, switched or removed.
    pub fn refresh(&self) {
        self.inner.refresh.store(true, Ordering::SeqCst);
    }

    /// The engine the host starts runs with. For tests and for callers that start a run by hand.
    pub fn engine_for_tests(&self) -> &Arc<WorkflowEngine> {
        &self.inner.engine
    }

    /// One round of everything, now. For tests and for a command that should not wait for the next beat.
    pub async fn beat_now(&self) -> Result<()> {
        self.inner.beat().await
    }

    /// The secret in a workflow's webhook address, made when first asked.
    pub async fn webhook_token(&self, workflow: WorkflowId) -> Result<String> {
        self.inner.secret_or_create(token_key(workflow)).await
    }

    /// Replace the secret in a workflow's webhook address: the old address stops working.
    pub async fn reset_webhook_token(&self, workflow: WorkflowId) -> Result<String> {
        self.inner.forget(token_key(workflow)).await?;
        self.webhook_token(workflow).await
    }

    /// The secret a sender signs its requests with (`X-Hub-Signature-256`), made when first asked.
    pub async fn signing_secret(&self, workflow: WorkflowId) -> Result<String> {
        self.inner.secret_or_create(signing_key(workflow)).await
    }

    /// Stop everything the host started and wait for it.
    pub async fn shutdown(&self) {
        self.cancel.cancel();
        let tasks: Vec<JoinHandle<()>> = std::mem::take(&mut *self.tasks.lock());
        for task in tasks {
            let _ = task.await;
        }
    }
}
