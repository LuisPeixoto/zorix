//! Running a workflow (ADR-0033 to ADR-0035).
//!
//! A run is a record that [`WorkflowEngine::advance`] moves forward one settled step at a time. It never holds a worker: a step
//! that waits (an execution, a person, a timer) leaves the run in the store, and the next `advance` picks it up. `advance` is
//! safe to call at any time and any number of times, so a restart, an event and a timer all use the same door.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{Map, Value, json};
use zorix_core::{
    Ask, Clock, Error, Fired, MAX_ACTIVE_STEPS, MAX_CHAIN, MAX_LOOP_ITEMS, Result, RunState, Service, Step, StepKind,
    StepRun, StepState, TenantId, Then, Timestamp, WaitDecision, WaitId, WaitKind, WaitStatus, WorkflowId, WorkflowRun,
    WorkflowRunId, WorkflowStore, WorkflowWait, redact, wait_fingerprint,
};

use crate::expr::Expr;
use crate::graph::{Readiness, readiness};
use crate::launcher::{Outcome, RunStart, StepLauncher, Target, ToolCall, http_call, send_call};
use crate::schedule::next_time_of_day;
use crate::template::{render, render_prompt, render_value};

/// The most characters of a step's text kept in a run.
const MAX_OUTPUT_CHARS: usize = 50_000;

fn invalid(what: impl Into<String>) -> Error {
    Error::InvalidInput(what.into())
}

/// What starts a run.
#[derive(Clone, Debug)]
pub struct StartRequest {
    /// The workflow.
    pub workflow: WorkflowId,
    /// The trigger step that fired; the first manual one (or the first trigger) when absent.
    pub trigger: Option<String>,
    /// What the trigger carried.
    pub payload: Value,
    /// A test: the draft runs, and every action asks first.
    pub test: bool,
    /// How many workflows started each other to get here.
    pub chain: u32,
    /// The run that started this one.
    pub from_run: Option<WorkflowRunId>,
}

impl StartRequest {
    /// A run started by hand with a payload.
    pub fn manual(workflow: WorkflowId, payload: Value) -> Self {
        Self { workflow, trigger: None, payload, test: false, chain: 0, from_run: None }
    }
}

/// What a person decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Go ahead.
    Approve,
    /// Do not.
    Refuse,
    /// The answer to a question.
    Answer(String),
}

/// A person's answer to a wait.
#[derive(Clone, Debug)]
pub struct Decision {
    /// The fingerprint of what they were shown.
    pub fingerprint: String,
    /// What they decided.
    pub verdict: Verdict,
    /// Who.
    pub by: String,
    /// A note.
    pub note: Option<String>,
}

/// Runs workflows.
pub struct WorkflowEngine {
    store: Arc<dyn WorkflowStore>,
    launcher: Arc<dyn StepLauncher>,
    clock: Arc<dyn Clock>,
    tenant: TenantId,
    locks: Mutex<HashMap<WorkflowRunId, Arc<tokio::sync::Mutex<()>>>>,
}

fn clip(text: &str) -> String {
    if text.chars().count() <= MAX_OUTPUT_CHARS {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(MAX_OUTPUT_CHARS).collect();
    cut.push('…');
    cut
}

fn dollars(micros: u64) -> String {
    format!("${:.2}", micros as f64 / 1_000_000.0)
}

impl WorkflowEngine {
    /// An engine for one tenant.
    pub fn new(
        store: Arc<dyn WorkflowStore>,
        launcher: Arc<dyn StepLauncher>,
        clock: Arc<dyn Clock>,
        tenant: TenantId,
    ) -> Self {
        Self { store, launcher, clock, tenant, locks: Mutex::new(HashMap::new()) }
    }

    /// The store.
    pub fn store(&self) -> &Arc<dyn WorkflowStore> {
        &self.store
    }

    /// The tenant.
    pub fn tenant(&self) -> &TenantId {
        &self.tenant
    }

    fn lock_for(&self, id: WorkflowRunId) -> Arc<tokio::sync::Mutex<()>> {
        self.locks.lock().entry(id).or_default().clone()
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Starting

    /// Start a run and take it as far as it goes without waiting.
    pub async fn start(&self, request: StartRequest) -> Result<WorkflowRun> {
        if request.chain > MAX_CHAIN {
            return Err(invalid(format!("Workflows started {MAX_CHAIN} others in a row; this one is not started.")));
        }
        let workflow = self
            .store
            .workflow(&self.tenant, request.workflow)
            .await?
            .ok_or_else(|| Error::Missing(format!("workflow {}", request.workflow)))?;
        let (spec, version) = if request.test {
            (workflow.draft.clone(), None)
        } else {
            let live = workflow
                .live_version
                .ok_or_else(|| invalid("This workflow has no published version yet. Publish it, or run a test."))?;
            let found = self
                .store
                .version(&self.tenant, workflow.id, live)
                .await?
                .ok_or_else(|| Error::Storage("a workflow lost its newest version".into()))?;
            (found.spec, Some(live))
        };
        crate::check::validate(&spec).map_err(Error::InvalidInput)?;
        let trigger = match &request.trigger {
            Some(id) => spec.step(id).filter(|s| s.kind.is_trigger()),
            None => spec.triggers().find(|s| matches!(s.kind, StepKind::Manual)).or_else(|| spec.triggers().next()),
        }
        .ok_or_else(|| invalid("That trigger is not in the workflow."))?
        .clone();
        let now = self.clock.now();
        let mut steps: BTreeMap<String, StepRun> =
            spec.steps.iter().map(|s| (s.id.clone(), StepRun::default())).collect();
        for step in spec.triggers() {
            let run = steps.entry(step.id.clone()).or_default();
            if step.id == trigger.id {
                run.state = StepState::Done;
                run.started_at = Some(now);
                run.finished_at = Some(now);
                run.output = Some(json!({"payload": request.payload}));
                run.note = Some(format!("fired {}", fired_at(now)));
            } else {
                run.state = StepState::Skipped;
                run.note = Some("not fired".into());
            }
        }
        let run = WorkflowRun {
            id: WorkflowRunId::new(),
            tenant: self.tenant.clone(),
            workflow: workflow.id,
            slug: workflow.slug.clone(),
            version,
            test: request.test,
            spec: spec.clone(),
            fired: Fired {
                step: trigger.id.clone(),
                label: trigger.kind.trigger_label().unwrap_or_else(|| trigger.title.clone()),
                payload: request.payload,
                at: now,
            },
            state: RunState::Running,
            steps,
            cost_micros: 0,
            error: None,
            chain: request.chain,
            from_run: request.from_run,
            started_at: now,
            finished_at: None,
            revision: 0,
        };
        if !self.store.put_run(&self.tenant, &run, None).await? {
            return Err(Error::Storage("a new run could not be written".into()));
        }
        self.advance(run.id).await
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Moving forward

    /// Take a run as far as it can go now. Safe to call at any time.
    pub async fn advance(&self, id: WorkflowRunId) -> Result<WorkflowRun> {
        let lock = self.lock_for(id);
        let _held = lock.lock().await;
        let mut run =
            self.store.run(&self.tenant, id).await?.ok_or_else(|| Error::Missing(format!("workflow run {id}")))?;
        self.drive(&mut run).await?;
        if run.state.is_final() {
            drop(_held);
            self.locks.lock().remove(&id);
        }
        Ok(run)
    }

    /// Advance every run that has not ended. Returns how many were looked at.
    pub async fn tick(&self) -> Result<usize> {
        let active = self.store.active_runs(&self.tenant).await?;
        let mut count = 0;
        for run in active {
            if let Err(error) = self.advance(run.id).await {
                tracing::warn!(run = %run.id, "a workflow run could not advance: {error}");
            }
            count += 1;
        }
        Ok(count)
    }

    /// After a restart: an action that was in flight may or may not have happened, so it asks a person instead of running again
    /// (the same rule as an unknown tool outcome); then every run moves on. Returns how many actions were turned into questions.
    pub async fn recover(&self) -> Result<usize> {
        let mut asked = 0;
        for run in self.store.active_runs(&self.tenant).await? {
            let lock = self.lock_for(run.id);
            let _held = lock.lock().await;
            let Some(mut run) = self.store.run(&self.tenant, run.id).await? else { continue };
            let now = self.clock.now();
            let interrupted: Vec<Step> = run
                .spec
                .steps
                .iter()
                .filter(|s| {
                    matches!(s.kind, StepKind::Send { .. } | StepKind::Http { .. })
                        && run.steps[&s.id].state == StepState::Running
                })
                .cloned()
                .collect();
            for step in interrupted {
                let ctx = self.context(&run);
                let Ok(call) = self.action_call(&step, &ctx) else { continue };
                let shown = json!({"tool": call.tool, "arguments": call.arguments, "interrupted": true});
                let message = format!(
                    "{} was running when ZORIX stopped, so it may already have happened. Run it again?",
                    step.title
                );
                self.open_wait(&mut run, &step, WaitKind::Action, message, shown, None, Then::Refuse, None, None, now)
                    .await?;
                asked += 1;
            }
            self.persist(&mut run).await?;
            drop(_held);
            self.advance(run.id).await?;
        }
        Ok(asked)
    }

    async fn persist(&self, run: &mut WorkflowRun) -> Result<()> {
        if self.store.put_run(&self.tenant, run, Some(run.revision)).await? {
            run.revision += 1;
            Ok(())
        } else {
            Err(Error::Storage(format!("workflow run {} was changed by something else at the same time", run.id)))
        }
    }

    fn context(&self, run: &WorkflowRun) -> Value {
        let mut steps = Map::new();
        for (id, step) in &run.steps {
            if step.state == StepState::Done {
                steps.insert(
                    id.clone(),
                    json!({"output": step.output.clone().unwrap_or(Value::Null), "note": step.note}),
                );
            }
        }
        json!({
            "trigger": {"payload": run.fired.payload, "label": run.fired.label},
            "steps": steps,
            "run": {"id": run.id.to_string(), "cost_micros": run.cost_micros, "test": run.test},
        })
    }

    async fn drive(&self, run: &mut WorkflowRun) -> Result<()> {
        if run.state.is_final() {
            return Ok(());
        }
        let now = self.clock.now();
        let ids: Vec<String> = run.spec.steps.iter().map(|s| s.id.clone()).collect();

        for id in &ids {
            let (state, waiting_on) = {
                let step = &run.steps[id];
                (step.state, step.wait.is_some())
            };
            match state {
                StepState::Running => self.poll_execution(run, id).await?,
                StepState::Waiting if waiting_on => self.poll_wait(run, id, now).await?,
                StepState::Waiting => self.poll_execution(run, id).await?,
                _ => {}
            }
        }

        loop {
            if self.first_failure(run).is_some() {
                break;
            }
            let mut progressed = false;
            for id in &ids {
                if run.steps[id].state != StepState::Pending {
                    continue;
                }
                match readiness(&run.spec, &run.steps, id) {
                    Readiness::NotYet => {}
                    Readiness::Skip => {
                        let step = run.steps.get_mut(id).expect("step exists");
                        step.state = StepState::Skipped;
                        step.note = Some("branch not taken".into());
                        progressed = true;
                    }
                    Readiness::Ready => {
                        let active = run.steps.values().filter(|s| s.state == StepState::Running).count();
                        if active >= MAX_ACTIVE_STEPS {
                            continue;
                        }
                        self.start_step(run, id, now).await?;
                        progressed = true;
                    }
                }
                if self.first_failure(run).is_some() {
                    break;
                }
            }
            if !progressed {
                break;
            }
        }

        run.cost_micros = run.steps.values().map(|s| s.cost_micros).sum();

        if let Some((id, error)) = self.first_failure(run) {
            self.end_failed(run, &id, error, now).await?;
        } else if run.steps.values().all(|s| s.state.is_settled()) {
            run.state = RunState::Completed;
            run.finished_at = Some(now);
        } else if run.steps.values().any(|s| matches!(s.state, StepState::Running)) {
            run.state = RunState::Running;
        } else {
            run.state = RunState::Waiting;
        }
        self.persist(run).await
    }

    fn first_failure(&self, run: &WorkflowRun) -> Option<(String, String)> {
        run.spec.steps.iter().find_map(|s| {
            let step = &run.steps[&s.id];
            (step.state == StepState::Failed).then(|| (s.id.clone(), step.error.clone().unwrap_or_default()))
        })
    }

    async fn end_failed(&self, run: &mut WorkflowRun, failed: &str, error: String, now: Timestamp) -> Result<()> {
        let title = run.spec.step(failed).map(|s| s.title.clone()).unwrap_or_else(|| failed.to_string());
        self.stop_the_rest(run, &format!("stopped when “{title}” failed"), now).await?;
        run.state = RunState::Failed;
        run.error = Some(format!("{title}: {error}"));
        run.finished_at = Some(now);
        Ok(())
    }

    /// Cancel what is still going and skip what has not started.
    async fn stop_the_rest(&self, run: &mut WorkflowRun, why: &str, now: Timestamp) -> Result<()> {
        let ids: Vec<String> = run.steps.keys().cloned().collect();
        for id in ids {
            let (state, executions, wait) = {
                let step = &run.steps[&id];
                (
                    step.state,
                    step.execution.into_iter().chain(step.executions.iter().copied()).collect::<Vec<_>>(),
                    step.wait,
                )
            };
            match state {
                StepState::Pending | StepState::Running | StepState::Waiting => {
                    for execution in executions {
                        let _ = self.launcher.cancel(execution, why).await;
                    }
                    if let Some(wait) = wait {
                        let _ = self
                            .store
                            .settle_wait(&self.tenant, wait, WaitDecision::of(WaitStatus::Cancelled), now)
                            .await?;
                    }
                    let step = run.steps.get_mut(&id).expect("step exists");
                    step.state = StepState::Skipped;
                    step.note = Some(why.to_string());
                    step.finished_at = Some(now);
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Cancel a run: what is going is cancelled, what is waiting is closed, and nothing else starts.
    pub async fn cancel(&self, id: WorkflowRunId, by: &str) -> Result<WorkflowRun> {
        let lock = self.lock_for(id);
        let _held = lock.lock().await;
        let mut run =
            self.store.run(&self.tenant, id).await?.ok_or_else(|| Error::Missing(format!("workflow run {id}")))?;
        if run.state.is_final() {
            return Ok(run);
        }
        let now = self.clock.now();
        self.stop_the_rest(&mut run, &format!("cancelled by {by}"), now).await?;
        run.state = RunState::Cancelled;
        run.finished_at = Some(now);
        self.persist(&mut run).await?;
        Ok(run)
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Polling what steps wait for

    async fn poll_execution(&self, run: &mut WorkflowRun, id: &str) -> Result<()> {
        let (single, many) = {
            let step = &run.steps[id];
            (step.execution, step.executions.clone())
        };
        let now = self.clock.now();
        if let Some(execution) = single {
            match self.launcher.outcome(execution).await {
                Ok(outcome) => self.apply_outcome(run, id, execution, outcome, now),
                Err(error) => self.fail(run, id, format!("The run could not be read: {error}"), now),
            }
        } else if !many.is_empty() {
            let mut outcomes = Vec::new();
            for execution in &many {
                match self.launcher.outcome(*execution).await {
                    Ok(outcome) => outcomes.push(outcome),
                    Err(error) => return self.failed(run, id, format!("A run could not be read: {error}"), now),
                }
            }
            self.apply_loop(run, id, &many, outcomes, now);
        }
        Ok(())
    }

    fn fail(&self, run: &mut WorkflowRun, id: &str, error: String, now: Timestamp) {
        let step = run.steps.get_mut(id).expect("step exists");
        step.state = StepState::Failed;
        step.note = None;
        step.error = Some(error);
        step.finished_at = Some(now);
    }

    fn failed(&self, run: &mut WorkflowRun, id: &str, error: String, now: Timestamp) -> Result<()> {
        self.fail(run, id, error, now);
        Ok(())
    }

    fn done(&self, run: &mut WorkflowRun, id: &str, output: Value, note: Option<String>, now: Timestamp) {
        let step = run.steps.get_mut(id).expect("step exists");
        step.state = StepState::Done;
        step.output = Some(output);
        step.note = note;
        step.error = None;
        step.finished_at = Some(now);
    }

    fn apply_outcome(
        &self,
        run: &mut WorkflowRun,
        id: &str,
        execution: zorix_core::ExecutionId,
        outcome: Outcome,
        now: Timestamp,
    ) {
        match outcome {
            Outcome::Running { cost_micros } => {
                let step = run.steps.get_mut(id).expect("step exists");
                step.cost_micros = cost_micros;
                step.state = StepState::Running;
                step.note = None;
            }
            Outcome::WaitingForPerson { cost_micros } => {
                let step = run.steps.get_mut(id).expect("step exists");
                step.cost_micros = cost_micros;
                step.state = StepState::Waiting;
                step.note = Some("waiting for a person".into());
            }
            Outcome::Completed { text, structured, cost_micros, steps } => {
                run.steps.get_mut(id).expect("step exists").cost_micros = cost_micros;
                let output =
                    redact(json!({"text": clip(&text), "structured": structured, "execution": execution.to_string()}));
                self.done(
                    run,
                    id,
                    output,
                    Some(format!("{steps} step{} · {}", if steps == 1 { "" } else { "s" }, dollars(cost_micros))),
                    now,
                );
            }
            Outcome::Failed { reason, cost_micros } => {
                run.steps.get_mut(id).expect("step exists").cost_micros = cost_micros;
                self.fail(run, id, reason, now);
            }
            Outcome::Cancelled => self.fail(run, id, "The run was cancelled.".into(), now),
        }
    }

    fn apply_loop(
        &self,
        run: &mut WorkflowRun,
        id: &str,
        executions: &[zorix_core::ExecutionId],
        outcomes: Vec<Outcome>,
        now: Timestamp,
    ) {
        let cost: u64 = outcomes
            .iter()
            .map(|o| match o {
                Outcome::Running { cost_micros }
                | Outcome::WaitingForPerson { cost_micros }
                | Outcome::Completed { cost_micros, .. }
                | Outcome::Failed { cost_micros, .. } => *cost_micros,
                Outcome::Cancelled => 0,
            })
            .sum();
        run.steps.get_mut(id).expect("step exists").cost_micros = cost;
        if let Some(reason) = outcomes.iter().find_map(|o| match o {
            Outcome::Failed { reason, .. } => Some(reason.clone()),
            Outcome::Cancelled => Some("A run was cancelled.".into()),
            _ => None,
        }) {
            return self.fail(run, id, reason, now);
        }
        if outcomes.iter().any(|o| matches!(o, Outcome::WaitingForPerson { .. })) {
            let step = run.steps.get_mut(id).expect("step exists");
            step.state = StepState::Waiting;
            step.note = Some("waiting for a person".into());
            return;
        }
        if outcomes.iter().any(|o| matches!(o, Outcome::Running { .. })) {
            let step = run.steps.get_mut(id).expect("step exists");
            step.state = StepState::Running;
            step.note = None;
            return;
        }
        let items: Vec<Value> = outcomes
            .into_iter()
            .zip(executions)
            .map(|(o, e)| match o {
                Outcome::Completed { text, structured, .. } => {
                    json!({"text": clip(&text), "structured": structured, "execution": e.to_string()})
                }
                _ => Value::Null,
            })
            .collect();
        let note = format!("{} items · {}", items.len(), dollars(cost));
        self.done(run, id, redact(json!({"items": items})), Some(note), now);
    }

    async fn poll_wait(&self, run: &mut WorkflowRun, id: &str, now: Timestamp) -> Result<()> {
        let Some(wait_id) = run.steps[id].wait else { return Ok(()) };
        let Some(mut wait) = self.store.wait(&self.tenant, wait_id).await? else {
            self.fail(run, id, "The request this step was waiting on was lost.".into(), now);
            return Ok(());
        };
        if wait.status == WaitStatus::Pending {
            if wait.due_at.is_some_and(|due| due <= now) {
                let status = if wait.kind == WaitKind::Timer { WaitStatus::Elapsed } else { WaitStatus::TimedOut };
                match self.store.settle_wait(&self.tenant, wait_id, WaitDecision::of(status), now).await? {
                    Some(settled) => wait = settled,
                    None => {
                        if let Some(fresh) = self.store.wait(&self.tenant, wait_id).await? {
                            wait = fresh;
                        }
                    }
                }
            } else {
                self.remind(&wait, run, now).await?;
                return Ok(());
            }
        }
        if wait.status == WaitStatus::Pending {
            return Ok(());
        }
        self.apply_wait(run, id, wait, now).await
    }

    async fn remind(&self, wait: &WorkflowWait, run: &WorkflowRun, now: Timestamp) -> Result<()> {
        if wait.reminded || !wait.remind_at.is_some_and(|at| at <= now) {
            return Ok(());
        }
        // Mark first: a reminder that fails is not retried every few seconds.
        self.store.mark_reminded(&self.tenant, wait.id).await?;
        let Some(StepKind::Approval { remind: Some(remind), .. }) = run.spec.step(&wait.step).map(|s| s.kind.clone())
        else {
            return Ok(());
        };
        let text = format!("Reminder: “{}” is waiting for a decision in ZORIX.", wait.title);
        let call = send_call(Service::Slack, Some(&remind.connection), &remind.channel, &text);
        if let Err(error) = self.launcher.call_tool(&call).await {
            tracing::warn!(wait = %wait.id, "a reminder was not sent: {error}");
        }
        Ok(())
    }

    async fn apply_wait(&self, run: &mut WorkflowRun, id: &str, wait: WorkflowWait, now: Timestamp) -> Result<()> {
        let by = wait.decided_by.clone().unwrap_or_else(|| "someone".into());
        match (wait.kind, wait.status) {
            (_, WaitStatus::Cancelled) => self.fail(run, id, "The request was cancelled.".into(), now),
            (WaitKind::Timer, _) => self.done(run, id, json!({}), None, now),
            (WaitKind::Approval, WaitStatus::Approved) => self.done(
                run,
                id,
                json!({"approved": true, "by": by, "note": wait.note}),
                Some(format!("approved by {by}")),
                now,
            ),
            (WaitKind::Approval, WaitStatus::TimedOut) if wait.then == Then::Continue => {
                self.done(run, id, json!({"approved": true, "timed_out": true}), Some("no answer: went on".into()), now)
            }
            (WaitKind::Question, WaitStatus::Answered) => {
                let answer = wait.answer.clone().unwrap_or_default();
                self.done(run, id, json!({"answer": clip(&answer), "by": by}), Some(format!("answered by {by}")), now)
            }
            (WaitKind::Question, WaitStatus::TimedOut) if wait.then == Then::Continue => self.done(
                run,
                id,
                json!({"answer": wait.default_answer.clone().unwrap_or_default(), "timed_out": true}),
                Some("no answer: used the default".into()),
                now,
            ),
            (WaitKind::Action, WaitStatus::Approved) => {
                let call = ToolCall {
                    tool: wait.shown["tool"].as_str().unwrap_or_default().to_string(),
                    arguments: wait.shown["arguments"].clone(),
                };
                self.execute_action(run, id, call, now).await?;
            }
            (_, WaitStatus::TimedOut) => {
                self.fail(run, id, "Nobody answered in time, so the step was refused.".into(), now)
            }
            (_, WaitStatus::Refused) => {
                let note = wait.note.map(|n| format!(" ({n})")).unwrap_or_default();
                self.fail(run, id, format!("Refused by {by}{note}."), now)
            }
            (_, other) => self.fail(run, id, format!("The request ended as “{}”.", other.as_str()), now),
        }
        Ok(())
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Starting steps

    fn remaining_budget(&self, run: &WorkflowRun) -> Option<u64> {
        run.spec
            .budget_micros
            .map(|budget| budget.saturating_sub(run.steps.values().map(|s| s.cost_micros).sum::<u64>()))
    }

    fn metadata(&self, run: &WorkflowRun, step: &str) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("workflow".to_string(), run.slug.to_string()),
            ("workflow_run".to_string(), run.id.to_string()),
            ("workflow_step".to_string(), step.to_string()),
            ("workflow_test".to_string(), run.test.to_string()),
        ])
    }

    fn cap(&self, run: &WorkflowRun, own: Option<u64>) -> std::result::Result<Option<u64>, String> {
        match (own, self.remaining_budget(run)) {
            (_, Some(0)) => Err("The workflow's budget is used up.".into()),
            (Some(a), Some(b)) => Ok(Some(a.min(b))),
            (a, b) => Ok(a.or(b)),
        }
    }

    async fn start_step(&self, run: &mut WorkflowRun, id: &str, now: Timestamp) -> Result<()> {
        let step = run.spec.step(id).expect("step exists").clone();
        {
            let record = run.steps.get_mut(id).expect("step exists");
            record.state = StepState::Running;
            record.started_at = Some(now);
        }
        let ctx = self.context(run);
        match &step.kind {
            StepKind::Agent { agent, prompt, version, model, max_cost_micros } => {
                let target = Target::Agent { agent: agent.clone(), version: *version };
                self.start_execution(run, id, target, prompt, model.clone(), *max_cost_micros, &ctx, now).await?;
            }
            StepKind::Team { team, prompt, model, max_cost_micros } => {
                let target = Target::Team { team: team.clone() };
                self.start_execution(run, id, target, prompt, model.clone(), *max_cost_micros, &ctx, now).await?;
            }
            StepKind::Loop { over, agent, prompt } => self.start_loop(run, id, over, agent, prompt, &ctx, now).await?,
            StepKind::Condition { expr } => match Expr::parse(expr) {
                Ok(parsed) => {
                    let value = parsed.test(&ctx);
                    self.done(run, id, json!({"value": value}), Some(if value { "yes" } else { "no" }.into()), now);
                }
                Err(error) => self.fail(run, id, format!("The condition is not valid: {error}"), now),
            },
            StepKind::Wait { for_ms, until } => {
                let due = match (for_ms, until) {
                    (Some(ms), _) => Ok(now.plus_millis(*ms)),
                    (None, Some(time)) => next_time_of_day(time, None, now),
                    _ => Err("A wait needs a duration or a time.".into()),
                };
                match due {
                    Ok(due) => {
                        let message = format!("Waiting until {}.", due.to_rfc3339());
                        self.open_wait(
                            run,
                            &step,
                            WaitKind::Timer,
                            message,
                            json!({}),
                            Some(due),
                            Then::Continue,
                            None,
                            None,
                            now,
                        )
                        .await?;
                    }
                    Err(error) => self.fail(run, id, error, now),
                }
            }
            StepKind::Approval { message, show, timeout_ms, then, remind } => {
                match self.approval_parts(&ctx, message, show) {
                    Ok((message, shown)) => {
                        let due = timeout_ms.map(|ms| now.plus_millis(ms));
                        let remind_at = remind.as_ref().map(|r| now.plus_millis(r.after_ms));
                        self.open_wait(
                            run,
                            &step,
                            WaitKind::Approval,
                            message,
                            shown,
                            due,
                            *then,
                            None,
                            remind_at,
                            now,
                        )
                        .await?;
                    }
                    Err(error) => self.fail(run, id, error, now),
                }
            }
            StepKind::Question { question, default, timeout_ms, then } => match render(question, &ctx) {
                Ok(message) => {
                    let due = timeout_ms.map(|ms| now.plus_millis(ms));
                    self.open_wait(
                        run,
                        &step,
                        WaitKind::Question,
                        message,
                        json!({}),
                        due,
                        *then,
                        default.clone(),
                        None,
                        now,
                    )
                    .await?;
                }
                Err(error) => self.fail(run, id, error.to_string(), now),
            },
            StepKind::Send { .. } | StepKind::Http { .. } => match self.action_call(&step, &ctx) {
                Ok(call) => {
                    let ask = match &step.kind {
                        StepKind::Send { ask, .. } | StepKind::Http { ask, .. } => *ask,
                        _ => Ask::Always,
                    };
                    if ask == Ask::Never && !run.test {
                        self.execute_action(run, id, call, now).await?;
                    } else {
                        let message = describe_call(&step, &call);
                        let shown = json!({"tool": call.tool, "arguments": call.arguments});
                        self.open_wait(
                            run,
                            &step,
                            WaitKind::Action,
                            message,
                            shown,
                            None,
                            Then::Refuse,
                            None,
                            None,
                            now,
                        )
                        .await?;
                    }
                }
                Err(error) => self.fail(run, id, error, now),
            },
            // A trigger is settled when the run starts.
            _ => {
                let record = run.steps.get_mut(id).expect("step exists");
                record.state = StepState::Skipped;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn start_execution(
        &self,
        run: &mut WorkflowRun,
        id: &str,
        target: Target,
        prompt: &str,
        model: Option<String>,
        own_cap: Option<u64>,
        ctx: &Value,
        now: Timestamp,
    ) -> Result<()> {
        let prompt = match render_prompt(prompt, ctx) {
            Ok(p) => p,
            Err(error) => return self.failed(run, id, error.to_string(), now),
        };
        let cap = match self.cap(run, own_cap) {
            Ok(cap) => cap,
            Err(error) => return self.failed(run, id, error, now),
        };
        let start = RunStart {
            target,
            prompt,
            input: None,
            model,
            max_cost_micros: cap,
            key: format!("wf-{}-{id}", run.id),
            metadata: self.metadata(run, id),
            access: Vec::new(),
            max_active_ms: None,
            autonomy: None,
        };
        match self.launcher.start(start).await {
            Ok(execution) => {
                run.steps.get_mut(id).expect("step exists").execution = Some(execution);
            }
            Err(error) => self.fail(run, id, error, now),
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn start_loop(
        &self,
        run: &mut WorkflowRun,
        id: &str,
        over: &str,
        agent: &zorix_core::AgentSlug,
        prompt: &str,
        ctx: &Value,
        now: Timestamp,
    ) -> Result<()> {
        let list = match render_value(over, ctx).or_else(|_| render_value(&format!("{{{{{over}}}}}"), ctx)) {
            Ok(Value::Array(items)) => items,
            Ok(_) => return self.failed(run, id, "The loop needs a list, and that value is not one.".into(), now),
            Err(error) => return self.failed(run, id, error.to_string(), now),
        };
        if list.len() > MAX_LOOP_ITEMS {
            return self.failed(
                run,
                id,
                format!("The list has {} items; a loop runs {MAX_LOOP_ITEMS} at most.", list.len()),
                now,
            );
        }
        if list.is_empty() {
            self.done(run, id, json!({"items": []}), Some("nothing to do".into()), now);
            return Ok(());
        }
        let mut started = Vec::new();
        for (index, item) in list.iter().enumerate() {
            let mut local = ctx.clone();
            local["item"] = item.clone();
            local["index"] = json!(index);
            let text = match render_prompt(prompt, &local) {
                Ok(t) => t,
                Err(error) => return self.failed(run, id, error.to_string(), now),
            };
            let cap = match self.cap(run, None) {
                Ok(cap) => cap.map(|c| c / list.len() as u64),
                Err(error) => return self.failed(run, id, error, now),
            };
            let start = RunStart {
                target: Target::Agent { agent: agent.clone(), version: None },
                prompt: text,
                input: None,
                model: None,
                max_cost_micros: cap,
                key: format!("wf-{}-{id}-{index}", run.id),
                metadata: self.metadata(run, id),
                access: Vec::new(),
                max_active_ms: None,
                autonomy: None,
            };
            match self.launcher.start(start).await {
                Ok(execution) => started.push(execution),
                Err(error) => {
                    for execution in started {
                        let _ = self.launcher.cancel(execution, "the loop could not start").await;
                    }
                    return self.failed(run, id, error, now);
                }
            }
        }
        run.steps.get_mut(id).expect("step exists").executions = started;
        Ok(())
    }

    fn approval_parts(
        &self,
        ctx: &Value,
        message: &str,
        show: &[String],
    ) -> std::result::Result<(String, Value), String> {
        let message = render(message, ctx).map_err(|e| e.to_string())?;
        let mut shown = Map::new();
        for path in show {
            let value = render_value(&format!("{{{{{path}}}}}"), ctx).map_err(|e| e.to_string())?;
            shown.insert(path.clone(), redact(value));
        }
        Ok((message, Value::Object(shown)))
    }

    fn action_call(&self, step: &Step, ctx: &Value) -> std::result::Result<ToolCall, String> {
        let fill = |t: &str| render(t, ctx).map_err(|e| e.to_string());
        match &step.kind {
            StepKind::Send { service, connection, channel, text, .. } => {
                Ok(send_call(*service, connection.as_deref(), &fill(channel)?, &fill(text)?))
            }
            StepKind::Http { method, url, body, .. } => {
                let body = body.as_deref().map(fill).transpose()?;
                Ok(http_call(method, &fill(url)?, body.as_deref()))
            }
            _ => Err("That step is not an action.".into()),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn open_wait(
        &self,
        run: &mut WorkflowRun,
        step: &Step,
        kind: WaitKind,
        message: String,
        shown: Value,
        due_at: Option<Timestamp>,
        then: Then,
        default_answer: Option<String>,
        remind_at: Option<Timestamp>,
        now: Timestamp,
    ) -> Result<()> {
        let fingerprint = wait_fingerprint(&message, &shown, &step.hash());
        let wait = WorkflowWait {
            id: WaitId::new(),
            tenant: self.tenant.clone(),
            run: run.id,
            workflow: run.workflow,
            step: step.id.clone(),
            kind,
            title: step.title.clone(),
            message,
            shown,
            fingerprint,
            status: WaitStatus::Pending,
            due_at,
            then,
            default_answer,
            remind_at,
            reminded: false,
            created_at: now,
            decided_at: None,
            decided_by: None,
            answer: None,
            note: None,
        };
        self.store.put_wait(&self.tenant, &wait).await?;
        let record = run.steps.get_mut(&step.id).expect("step exists");
        record.state = StepState::Waiting;
        record.wait = Some(wait.id);
        record.started_at.get_or_insert(now);
        record.note = Some(match kind {
            WaitKind::Timer => "waiting".into(),
            WaitKind::Approval => "waiting for approval".into(),
            WaitKind::Question => "waiting for an answer".into(),
            WaitKind::Action => "waiting to be allowed".into(),
        });
        Ok(())
    }

    /// Make a call. The step is written as running first, so a crash in the middle leaves a mark and recovery asks a person
    /// instead of repeating a message that may have been sent.
    async fn execute_action(&self, run: &mut WorkflowRun, id: &str, call: ToolCall, now: Timestamp) -> Result<()> {
        {
            let record = run.steps.get_mut(id).expect("step exists");
            record.state = StepState::Running;
            record.note = Some("running".into());
        }
        self.persist(run).await?;
        match self.launcher.call_tool(&call).await {
            Ok(output) => {
                let finished = self.clock.now().max(now);
                self.done(run, id, redact(json!({"tool": call.tool, "result": output})), Some("done".into()), finished);
            }
            Err(error) => {
                let finished = self.clock.now().max(now);
                self.fail(run, id, error, finished)
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------------------------------------------------
    // People

    /// Answer a wait. The fingerprint must be the one the person was shown.
    pub async fn decide(&self, id: WaitId, decision: Decision) -> Result<WorkflowWait> {
        let wait = self.store.wait(&self.tenant, id).await?.ok_or_else(|| Error::Missing(format!("wait {id}")))?;
        if wait.status != WaitStatus::Pending {
            return Err(invalid("That was already answered."));
        }
        if wait.fingerprint != decision.fingerprint {
            return Err(invalid("What you were shown has changed since you looked. Open it again before you answer."));
        }
        let (status, answer) = match (wait.kind, &decision.verdict) {
            (WaitKind::Approval | WaitKind::Action, Verdict::Approve) => (WaitStatus::Approved, None),
            (WaitKind::Approval | WaitKind::Action | WaitKind::Question, Verdict::Refuse) => {
                (WaitStatus::Refused, None)
            }
            (WaitKind::Question, Verdict::Answer(text)) => {
                if text.trim().is_empty() || text.chars().count() > 4000 {
                    return Err(invalid("An answer has 1 to 4000 characters."));
                }
                (WaitStatus::Answered, Some(text.clone()))
            }
            (WaitKind::Timer, _) => return Err(invalid("A timer is not answered by a person.")),
            (WaitKind::Question, Verdict::Approve) => {
                return Err(invalid("A question needs an answer, not an approval."));
            }
            (_, Verdict::Answer(_)) => return Err(invalid("This asks for approval, not for an answer.")),
        };
        let now = self.clock.now();
        let settled = self
            .store
            .settle_wait(
                &self.tenant,
                id,
                WaitDecision { status, by: Some(decision.by), answer, note: decision.note },
                now,
            )
            .await?
            .ok_or_else(|| invalid("That was already answered."))?;
        self.advance(wait.run).await?;
        Ok(settled)
    }
}

fn fired_at(at: Timestamp) -> String {
    let text = at.to_rfc3339();
    text.get(11..16).unwrap_or("now").to_string()
}

fn describe_call(step: &Step, call: &ToolCall) -> String {
    match &step.kind {
        StepKind::Send { channel, .. } => {
            format!("{} — send a message to {}", step.title, call.arguments["channel"].as_str().unwrap_or(channel))
        }
        StepKind::Http { method, .. } => {
            format!("{} — {method} {}", step.title, call.arguments["url"].as_str().unwrap_or_default())
        }
        _ => step.title.clone(),
    }
}
