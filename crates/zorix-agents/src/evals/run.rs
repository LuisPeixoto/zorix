//! Running a suite: each case through the AgentRunner, with writes dropped and approvals refused, then checked (and judged).

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use zorix_core::{
    Action, Agent, AgentStore, AgentVersion, ApprovalDecision, Clock, EVAL_LABEL, EvalResult, EvalRun, EvalRunId,
    ExecutionEngine, ExecutionState, Grants, MemoryOptions, ModelGateway, Permission, StoredCase, SystemClock,
};

use super::checks::run_checks;
use super::judge::judge;
use crate::runner::{AgentAsk, ModelResolver, RunnerError, agent_grants, prepare};

/// The reason an evaluation gives when it refuses an approval.
pub const REFUSAL: &str = "evaluations refuse approvals";

/// How to run a suite.
#[derive(Clone, Debug)]
pub struct EvalOptions {
    /// The model to try (the agent's default when `None`).
    pub model: Option<String>,
    /// Also drop `network` permissions.
    pub offline: bool,
    /// Only cases with one of these tags (all when empty).
    pub tags: Vec<String>,
    /// How many cases run at once.
    pub concurrency: usize,
    /// The longest one case may take.
    pub case_timeout: Duration,
}

impl Default for EvalOptions {
    fn default() -> Self {
        Self { model: None, offline: false, tags: Vec::new(), concurrency: 2, case_timeout: Duration::from_secs(300) }
    }
}

/// Called with each case result as soon as it is known.
pub type OnResult = Arc<dyn Fn(&EvalResult) + Send + Sync>;

/// What it takes to run evaluations.
#[derive(Clone)]
pub struct Evaluator {
    /// Runs the cases.
    pub engine: Arc<dyn ExecutionEngine>,
    /// Keeps the results.
    pub store: Arc<dyn AgentStore>,
    /// Says which models a name stands for.
    pub resolver: Arc<dyn ModelResolver>,
    /// Answers the judge's questions, when a case has a judge.
    pub judge: Option<Arc<dyn ModelGateway>>,
}

/// The agent's grants for an evaluation: without writes unless the case allows them, and without the network when offline.
fn eval_grants(agent: &AgentVersion, allow_writes: bool, offline: bool) -> Result<Grants, RunnerError> {
    let all = agent_grants(&agent.spec)?;
    Ok(Grants::new(
        all.iter()
            .filter(|p| allow_writes || p.action != Action::Write)
            .filter(|p| !offline || p.action != Action::Network)
            .cloned()
            .collect(),
    ))
}

fn failed(case: &str, why: impl Into<String>) -> EvalResult {
    EvalResult {
        case: case.to_string(),
        execution: None,
        passed: false,
        checks: Vec::new(),
        judge: None,
        state: "not run".into(),
        steps: 0,
        tool_calls: 0,
        cost_micros: 0,
        error: Some(why.into()),
    }
}

impl Evaluator {
    /// Run the cases of an agent version with one model. Results are saved as they come in.
    pub async fn run(
        &self,
        agent: &Agent,
        version: &AgentVersion,
        cases: Vec<StoredCase>,
        options: EvalOptions,
        cancel: CancellationToken,
        on_result: Option<OnResult>,
    ) -> Result<EvalRun, RunnerError> {
        let chosen: Vec<StoredCase> = cases
            .into_iter()
            .filter(|c| options.tags.is_empty() || c.case.tags.iter().any(|t| options.tags.contains(t)))
            .collect();
        if chosen.is_empty() {
            return Err(RunnerError::Refused(if options.tags.is_empty() {
                format!("{} has no evaluation cases yet.", agent.slug)
            } else {
                format!("None of the cases of {} has one of the tags {}.", agent.slug, options.tags.join(", "))
            }));
        }
        let model = options.model.clone().unwrap_or_else(|| version.spec.models.default.clone());
        let mut record = EvalRun {
            id: EvalRunId::new(),
            agent: agent.id,
            version: version.version,
            model: model.clone(),
            status: "running".into(),
            started_at: SystemClock.now(),
            finished_at: None,
            results: Vec::new(),
        };
        self.store.save_eval_run(&agent.tenant, &record).await?;

        let gate = Arc::new(Semaphore::new(options.concurrency.clamp(1, 8)));
        let mut tasks: JoinSet<(usize, EvalResult)> = JoinSet::new();
        for (index, stored) in chosen.iter().cloned().enumerate() {
            let (me, agent, version, options, cancel, gate, model) = (
                self.clone(),
                agent.clone(),
                version.clone(),
                options.clone(),
                cancel.clone(),
                gate.clone(),
                model.clone(),
            );
            let run_id = record.id;
            tasks.spawn(async move {
                let _turn = gate.acquire().await;
                let result = if cancel.is_cancelled() {
                    failed(&stored.case.name, "The evaluation was cancelled before this case started.")
                } else {
                    me.run_case(&agent, &version, &stored, &model, &options, run_id, &cancel).await
                };
                (index, result)
            });
        }
        let mut finished: Vec<Option<EvalResult>> = vec![None; chosen.len()];
        while let Some(done) = tasks.join_next().await {
            let Ok((index, result)) = done else { continue };
            if let Some(callback) = &on_result {
                callback(&result);
            }
            finished[index] = Some(result);
            record.results = finished.iter().flatten().cloned().collect();
            self.store.save_eval_run(&agent.tenant, &record).await?;
        }
        record.results = chosen
            .iter()
            .zip(finished)
            .map(|(stored, result)| result.unwrap_or_else(|| failed(&stored.case.name, "The case did not finish.")))
            .collect();
        record.status = if cancel.is_cancelled() { "cancelled" } else { "finished" }.into();
        record.finished_at = Some(SystemClock.now());
        self.store.save_eval_run(&agent.tenant, &record).await?;
        Ok(record)
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_case(
        &self,
        agent: &Agent,
        version: &AgentVersion,
        stored: &StoredCase,
        model: &str,
        options: &EvalOptions,
        run_id: EvalRunId,
        cancel: &CancellationToken,
    ) -> EvalResult {
        let case = &stored.case;
        let grants = match eval_grants(version, case.allow_writes, options.offline) {
            Ok(grants) => grants,
            Err(e) => return failed(&case.name, e.to_string()),
        };
        let mut ask = AgentAsk {
            prompt: case.prompt.clone(),
            input: case.input.clone(),
            model: Some(model.to_string()),
            narrow: Some(grants),
            memory: Some(MemoryOptions::none()),
            agent_memory: Some(false),
            ..AgentAsk::default()
        };
        ask.metadata.insert(EVAL_LABEL.into(), run_id.to_string());
        ask.metadata.insert("eval_case".into(), case.name.clone());
        let new = match prepare(agent, version, ask, &*self.store, &*self.resolver).await {
            Ok(new) => new,
            Err(e) => return failed(&case.name, e.to_string()),
        };
        let tenant = agent.tenant.clone();
        let created = match self.engine.create(new).await {
            Ok(created) => created,
            Err(e) => return failed(&case.name, e.to_string()),
        };
        if let Err(e) = self.engine.start(&tenant, created.id).await {
            return failed(&case.name, e.to_string());
        }

        let deadline = tokio::time::Instant::now() + options.case_timeout;
        let mut problem: Option<String> = None;
        loop {
            let waited = tokio::select! {
                _ = cancel.cancelled() => None,
                r = tokio::time::timeout_at(deadline, self.engine.settle(&tenant, created.id)) => Some(r),
            };
            match waited {
                None => {
                    let _ = self.engine.cancel(&tenant, created.id, Some("the evaluation was cancelled".into())).await;
                    problem = Some("The evaluation was cancelled.".into());
                    break;
                }
                Some(Err(_)) => {
                    let _ = self.engine.cancel(&tenant, created.id, Some("the case took too long".into())).await;
                    problem = Some(format!("The case took longer than {} seconds.", options.case_timeout.as_secs()));
                    break;
                }
                Some(Ok(Err(e))) => {
                    problem = Some(e.to_string());
                    break;
                }
                Some(Ok(Ok(execution))) => match execution.state {
                    // A sub-run that needs a person leaves its root waiting for children: the tree is answered as a whole.
                    ExecutionState::WaitingForApproval | ExecutionState::WaitingForChildren => {
                        if let Err(e) = self.answer_approvals(&tenant, created.id, &case.approve).await {
                            problem = Some(e);
                            break;
                        }
                    }
                    ExecutionState::Paused => {
                        problem = Some("The run was paused.".into());
                        break;
                    }
                    _ => break,
                },
            }
        }

        let detail = match self.engine.detail(&tenant, created.id).await {
            Ok(detail) => detail,
            Err(e) => return failed(&case.name, e.to_string()),
        };
        let mut checks = run_checks(case, &detail);
        let answer = detail.execution.result.as_ref().map(|r| r.text.clone());
        let mut verdict = None;
        let mut judge_error = None;
        if let (Some(judging), Some(gateway)) = (&case.judge, &self.judge) {
            match &answer {
                Some(text) => match judge(&**gateway, judging, case, text, cancel).await {
                    Ok(outcome) => verdict = Some(outcome),
                    Err(e) => judge_error = Some(e),
                },
                None => judge_error = Some("There was no answer to judge.".into()),
            }
        } else if case.judge.is_some() {
            judge_error = Some("This case has a judge, but no judge model is available.".into());
        }
        let judged_ok = match (&case.judge, &verdict) {
            (Some(j), Some(v)) => v.score >= j.pass_score,
            (Some(_), None) => false,
            (None, _) => true,
        };
        let passed = checks.iter().all(|c| c.passed) && judged_ok && problem.is_none();
        if let Some(why) = &judge_error {
            checks.push(zorix_core::CheckOutcome { check: "judged".into(), passed: false, detail: why.clone() });
        }
        EvalResult {
            case: case.name.clone(),
            execution: Some(created.id),
            passed,
            checks,
            judge: verdict,
            state: detail.execution.state.as_str().to_string(),
            steps: detail.execution.usage.model_calls,
            tool_calls: detail.tool_calls.len() as u32,
            cost_micros: detail.execution.usage.cost_micros,
            error: problem,
        }
    }

    /// Answer every pending approval anywhere in the tree of `root`: refuse it, unless the case named the tool and only a person
    /// could not be the one answering (a tool that needs a person is always refused).
    async fn answer_approvals(
        &self,
        tenant: &zorix_core::TenantId,
        root: zorix_core::ExecutionId,
        approve: &[String],
    ) -> Result<(), String> {
        let waiting = self.engine.pending_in_tree(tenant, root).await.map_err(|e| e.to_string())?;
        for item in waiting {
            let (id, approval) = (item.execution, item.approval);
            let events = self.engine.events(tenant, id, 0).await.map_err(|e| e.to_string())?;
            let person_only = events.iter().any(|e| {
                e.kind.name() == "execution.waiting_approval"
                    && e.payload.get("approval_id").and_then(Value::as_str) == Some(approval.id.to_string().as_str())
                    && e.payload.get("person_only") == Some(&Value::Bool(true))
            });
            let named = approve.iter().any(|p| Permission::use_tool(p).covers(&Permission::use_tool(&approval.tool)));
            let yes = named && !person_only;
            let decision = ApprovalDecision {
                approve: yes,
                decided_by: "evaluation".into(),
                note: (!yes).then(|| REFUSAL.to_string()),
                call_hash: Some(approval.call_hash.clone()),
                automatic: true,
            };
            self.engine.decide(tenant, id, approval.id, decision).await.map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
