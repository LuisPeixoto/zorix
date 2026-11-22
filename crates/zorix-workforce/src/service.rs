//! What the screens, the commands and the manager's tools ask of the workforce (ADR-0038, ADR-0039).
//!
//! The service holds the rules. A worker is a record over an agent; its figures are computed from projects, executions and
//! evaluations and never stored. It shapes the runs the workforce starts ([`RunShaper`]), starts a manager's plan and report,
//! and [`WorkforceService::reconcile`] keeps what runs in the background up to date: finished plans and reports, the schedule and the
//! budget notices. It is safe to call at any time and as often as wanted.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;
use zorix_core::{
    Agent, AgentSlug, AgentStore, AutonomyLevel, Avatar, Clock, Date, EVAL_LABEL, Error, EventKind, Execution,
    ExecutionId, ExecutionRepository, ExecutionState, ListFilter, OrgEvent, OrgEventDraft, OrgEventKind, OrgReport,
    OrgReportId, OrgSettings, OrgStore, Project, ProjectStore, ReportKind, ReportStatus, Result, Task, TaskState,
    TenantId, Timestamp, Worker, validate_org,
};
use zorix_projects::RunShaper;
use zorix_workflows::launcher::{Outcome, RunStart, StepLauncher, Target};
use zorix_workflows::schedule::{day_start, fires_between, local_date, week_start};

use crate::rules::{Shaped, is_near, left, on_time_counts, open_for, plan_prompt, report_prompt, shape_run};

const DAY_MS: i64 = 86_400_000;
/// A scheduled slot is run only when it is no older than this; an older one is recorded as missed.
const MISS_GRACE_MS: i64 = 5 * 60 * 1_000;
/// How many executions of the last two weeks are read to compute spend and status.
const EXECUTION_WINDOW: usize = 5_000;
/// How many days of a worker's spend the profile draws.
pub const SPARK_DAYS: usize = 7;

fn invalid(what: impl Into<String>) -> Error {
    Error::InvalidInput(what.into())
}

// ---------------------------------------------------------------------------------------------------------------------
// What a person sends

/// A worker to hire.
#[derive(Clone, Debug, Deserialize)]
pub struct NewWorker {
    /// The agent it is a record over.
    pub agent: AgentSlug,
    /// What it does.
    pub role: String,
    /// Its department's short name.
    pub department: String,
    /// Who it answers to: another worker, or you (none).
    #[serde(default)]
    pub reports_to: Option<AgentSlug>,
    /// A virtual manager.
    #[serde(default)]
    pub manager: bool,
    /// Its level; the organisation's default level when absent.
    #[serde(default)]
    pub autonomy: Option<AutonomyLevel>,
    /// The most it may spend in a week.
    #[serde(default)]
    pub weekly_budget_micros: Option<u64>,
    /// The most active time for a task, in milliseconds; two hours when absent.
    #[serde(default)]
    pub task_time_ms: Option<u64>,
    /// What always asks.
    #[serde(default)]
    pub still_ask: Vec<String>,
}

/// What a person may change about a worker. All of it is replaced; a screen sends what it showed.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct WorkerFields {
    /// What it does.
    pub role: String,
    /// Its department's short name.
    pub department: String,
    /// Who it answers to.
    #[serde(default)]
    pub reports_to: Option<AgentSlug>,
    /// A virtual manager.
    #[serde(default)]
    pub manager: bool,
    /// Its level.
    pub autonomy: AutonomyLevel,
    /// The most it may spend in a week.
    #[serde(default)]
    pub weekly_budget_micros: Option<u64>,
    /// The most active time for a task.
    pub task_time_ms: u64,
    /// What always asks.
    #[serde(default)]
    pub still_ask: Vec<String>,
    /// Whether the plan and report run on their own.
    #[serde(default)]
    pub schedule_enabled: bool,
}

impl WorkerFields {
    /// What a worker has now.
    pub fn of(worker: &Worker) -> Self {
        Self {
            role: worker.role.clone(),
            department: worker.department.clone(),
            reports_to: worker.reports_to.clone(),
            manager: worker.manager,
            autonomy: worker.autonomy,
            weekly_budget_micros: worker.weekly_budget_micros,
            task_time_ms: worker.task_time_ms,
            still_ask: worker.still_ask.clone(),
            schedule_enabled: worker.schedule.enabled,
        }
    }
}

/// What a person may change about the organisation. The notices already read are kept.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SettingsFields {
    /// The most all workers may spend in a week.
    #[serde(default)]
    pub weekly_budget_micros: Option<u64>,
    /// The level new workers start at.
    pub default_level: AutonomyLevel,
    /// The departments.
    pub departments: Vec<zorix_core::Department>,
}

// ---------------------------------------------------------------------------------------------------------------------
// What the screens read

/// What a worker or a manager's tree has done and has to do.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Figures {
    /// Open tasks.
    pub tasks: u32,
    /// Tasks done in the last 30 days on or before their due day, over the ones that had a due day.
    pub on_time_done: u32,
    /// The tasks that had a due day.
    pub on_time_all: u32,
    /// The latest finished evaluation: cases passed over cases (0 to 1). Not for a tree.
    pub eval: Option<f64>,
    /// Spent since Monday, in millionths of a dollar.
    pub week_micros: u64,
    /// Runs waiting for a person.
    pub waiting: u32,
}

impl Figures {
    /// The share on time (0 to 1), or none when no task had a due day.
    pub fn on_time(&self) -> Option<f64> {
        (self.on_time_all > 0).then(|| f64::from(self.on_time_done) / f64::from(self.on_time_all))
    }

    fn add(&mut self, other: &Figures) {
        self.tasks += other.tasks;
        self.on_time_done += other.on_time_done;
        self.on_time_all += other.on_time_all;
        self.week_micros += other.week_micros;
        self.waiting += other.waiting;
    }
}

/// A worker as the screens show it.
#[derive(Clone, Debug, Serialize)]
pub struct WorkerView {
    /// The record.
    pub worker: Worker,
    /// The agent's name.
    pub name: String,
    /// One sentence about the agent.
    pub description: String,
    /// How the agent looks.
    pub avatar: Avatar,
    /// `ok` working, `warn` waiting for a person, `err` stopped, none when idle.
    pub status: Option<&'static str>,
    /// What it did and has to do; for a worker with reports, everyone below it too.
    pub figures: Figures,
    /// The worker's own figures.
    pub own: Figures,
    /// Who reports to it.
    pub reports: Vec<AgentSlug>,
    /// What is left of its weekly budget.
    pub budget_left_micros: Option<u64>,
    /// The agent is archived: it still counts, but cannot run.
    pub archived: bool,
}

/// The card of you at the top of every chain.
#[derive(Clone, Debug, Default, Serialize)]
pub struct YouView {
    /// Tasks waiting for you to review.
    pub to_review: u32,
    /// Runs waiting for you to approve something.
    pub approvals: u32,
    /// Who answers to you.
    pub reports: Vec<AgentSlug>,
}

/// The whole organisation.
#[derive(Clone, Debug, Serialize)]
pub struct Overview {
    /// The settings and departments.
    pub settings: OrgSettings,
    /// The workers, by hiring time.
    pub workers: Vec<WorkerView>,
    /// You.
    pub you: YouView,
    /// Open tasks across the active projects.
    pub open_tasks: u32,
    /// Open tasks due this week.
    pub due_this_week: u32,
    /// Runs waiting for you.
    pub waiting_approvals: u32,
    /// Tasks waiting for your review.
    pub waiting_reviews: u32,
    /// Spent by everything since Monday.
    pub week_micros: u64,
    /// The organisation's weekly budget.
    pub budget_micros: Option<u64>,
    /// Runs going now, and the workers busy with them.
    pub running: u32,
    /// Workers with a run going.
    pub busy: u32,
    /// The Monday of this week.
    pub week: String,
}

/// An open task of a worker, for its page.
#[derive(Clone, Debug, Serialize)]
pub struct OpenTask {
    /// Its key.
    pub key: String,
    /// Its title.
    pub title: String,
    /// Its lane.
    pub state: TaskState,
    /// Its due day.
    pub due: Option<Date>,
    /// Whether the due day has gone.
    pub late: bool,
}

/// One line of what the workers did.
#[derive(Clone, Debug, Serialize)]
pub struct ActivityLine {
    /// A stable name for the line.
    pub id: String,
    /// When.
    pub at: Timestamp,
    /// `OK`, `INFO`, `WAIT`, `WARN` or `ERR`.
    pub level: &'static str,
    /// The worker, or `org`.
    pub source: String,
    /// One sentence.
    pub text: String,
    /// What it is: `suggestion`, `unattended`, `waiting`, `failed`, `plan`, `report`, `budget`, `org`.
    pub kind: &'static str,
    /// The run it came from.
    pub run: Option<ExecutionId>,
}

/// Something worth a person's eye.
#[derive(Clone, Debug, Serialize)]
pub struct Notice {
    /// A stable name.
    pub id: String,
    /// `suggestion`, `notice` or `report`.
    pub kind: &'static str,
    /// When.
    pub at: Timestamp,
    /// The worker.
    pub worker: Option<String>,
    /// A headline.
    pub title: String,
    /// More.
    pub text: String,
    /// The run it came from.
    pub run: Option<ExecutionId>,
}

/// A worker's page.
#[derive(Clone, Debug, Serialize)]
pub struct WorkerDetail {
    /// The worker.
    pub view: WorkerView,
    /// Spent each day for the last seven days, oldest first.
    pub spend_days: Vec<u64>,
    /// Spent in the same part of last week, for the change.
    pub spend_last_week_micros: u64,
    /// Its open tasks, soonest due first.
    pub tasks: Vec<OpenTask>,
    /// What it did this week.
    pub activity: Vec<ActivityLine>,
    /// Plans and reports it wrote, newest first.
    pub reports: Vec<OrgReport>,
    /// A plan or report is going.
    pub busy_with: Option<ReportKind>,
}

// ---------------------------------------------------------------------------------------------------------------------
// The service

/// A snapshot of what a screen needs, read once.
struct Facts {
    now: Timestamp,
    week: Timestamp,
    tasks: Vec<Task>,
    executions: Vec<Execution>,
    own_cost: BTreeMap<ExecutionId, u64>,
}

impl Facts {
    fn week_spend(&self, agent: &AgentSlug) -> u64 {
        self.executions
            .iter()
            .filter(|e| e.created_at >= self.week && e.agent.as_ref().is_some_and(|a| a.slug == *agent))
            .map(|e| self.own_cost.get(&e.id).copied().unwrap_or(0))
            .sum()
    }

    fn org_spend(&self) -> u64 {
        self.executions
            .iter()
            .filter(|e| e.created_at >= self.week && e.parent.is_none())
            .map(|e| e.usage.cost_micros)
            .sum()
    }

    fn waiting_of(&self, agent: &AgentSlug) -> u32 {
        self.executions
            .iter()
            .filter(|e| {
                e.state == ExecutionState::WaitingForApproval && e.agent.as_ref().is_some_and(|a| a.slug == *agent)
            })
            .count() as u32
    }

    fn status_of(&self, agent: &AgentSlug) -> Option<&'static str> {
        let mine: Vec<&Execution> =
            self.executions.iter().filter(|e| e.agent.as_ref().is_some_and(|a| a.slug == *agent)).collect();
        if mine.iter().any(|e| e.state == ExecutionState::WaitingForApproval) {
            return Some("warn");
        }
        let active = |s: ExecutionState| {
            matches!(
                s,
                ExecutionState::Queued
                    | ExecutionState::Running
                    | ExecutionState::WaitingForModel
                    | ExecutionState::WaitingForTool
                    | ExecutionState::WaitingForChildren
            )
        };
        if mine.iter().any(|e| active(e.state)) {
            return Some("ok");
        }
        let newest = mine.iter().max_by_key(|e| e.created_at)?;
        (newest.state == ExecutionState::Failed && self.now.millis() - newest.created_at.millis() < DAY_MS)
            .then_some("err")
    }
}

/// The workforce of one tenant.
pub struct WorkforceService {
    org: Arc<dyn OrgStore>,
    projects: Arc<dyn ProjectStore>,
    agents: Arc<dyn AgentStore>,
    executions: Arc<dyn ExecutionRepository>,
    launcher: Arc<dyn StepLauncher>,
    tenant: TenantId,
    clock: Arc<dyn Clock>,
    timezone: Option<String>,
    model: parking_lot::Mutex<Option<String>>,
    gate: Mutex<()>,
}

impl WorkforceService {
    /// A service for one tenant.
    pub fn new(
        org: Arc<dyn OrgStore>,
        projects: Arc<dyn ProjectStore>,
        agents: Arc<dyn AgentStore>,
        executions: Arc<dyn ExecutionRepository>,
        launcher: Arc<dyn StepLauncher>,
        tenant: TenantId,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            org,
            projects,
            agents,
            executions,
            launcher,
            tenant,
            clock,
            timezone: None,
            model: parking_lot::Mutex::new(None),
            gate: Mutex::new(()),
        }
    }

    /// Count weeks and days in this time zone (an IANA name) instead of the computer's.
    pub fn in_zone(mut self, zone: impl Into<String>) -> Self {
        self.timezone = Some(zone.into());
        self
    }

    /// Run plans and reports with this model instead of the agent's own. None leaves it to the agent.
    pub fn set_model(&self, model: Option<String>) {
        *self.model.lock() = model;
    }

    /// The tenant it works for.
    pub fn tenant(&self) -> &TenantId {
        &self.tenant
    }

    fn now(&self) -> Timestamp {
        self.clock.now()
    }

    fn zone(&self) -> Option<&str> {
        self.timezone.as_deref()
    }

    fn week_of(&self, at: Timestamp) -> Timestamp {
        week_start(at, self.zone()).unwrap_or(Timestamp(at.millis() - at.millis().rem_euclid(7 * DAY_MS)))
    }

    fn day_text(&self, at: Timestamp) -> String {
        local_date(at, self.zone()).unwrap_or_else(|_| "1970-01-01".into())
    }

    fn draft(
        &self,
        kind: OrgEventKind,
        worker: Option<&AgentSlug>,
        text: impl Into<String>,
        by: &str,
        run: Option<ExecutionId>,
    ) -> OrgEventDraft {
        OrgEventDraft { kind, worker: worker.cloned(), text: text.into(), by: by.into(), run, at: self.now() }
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Settings, hiring, editing

    /// The organisation's settings.
    pub async fn settings(&self) -> Result<OrgSettings> {
        self.org.settings(&self.tenant).await
    }

    /// Change the settings and the departments. A department with workers in it cannot go.
    pub async fn save_settings(&self, fields: SettingsFields, by: &str) -> Result<OrgSettings> {
        let _lock = self.gate.lock().await;
        let current = self.org.settings(&self.tenant).await?;
        let next = OrgSettings {
            weekly_budget_micros: fields.weekly_budget_micros,
            default_level: fields.default_level,
            departments: fields.departments,
            seen_until: current.seen_until,
        };
        let workers = self.org.workers(&self.tenant).await?;
        validate_org(&next, &workers)?;
        let event = self.draft(OrgEventKind::SettingsChanged, None, "The organisation's settings changed", by, None);
        self.org.put_settings(&self.tenant, &next, &[event]).await?;
        Ok(next)
    }

    /// Every worker, by hiring time.
    pub async fn workers(&self) -> Result<Vec<Worker>> {
        self.org.workers(&self.tenant).await
    }

    /// One worker.
    pub async fn worker(&self, agent: &AgentSlug) -> Result<Worker> {
        self.org
            .worker(&self.tenant, agent)
            .await?
            .ok_or_else(|| Error::Missing(format!("{agent} is not a worker. `zorix worker list` shows them.")))
    }

    async fn live_agent(&self, slug: &AgentSlug) -> Result<Agent> {
        let agent = self
            .agents
            .agent_by_slug(&self.tenant, slug)
            .await?
            .ok_or_else(|| Error::Missing(format!("There is no agent called {slug}.")))?;
        if agent.archived {
            return Err(invalid(format!("{slug} is archived, so it cannot be hired.")));
        }
        Ok(agent)
    }

    /// Make an agent a worker.
    pub async fn hire(&self, new: NewWorker, by: &str) -> Result<Worker> {
        let _lock = self.gate.lock().await;
        self.live_agent(&new.agent).await?;
        let settings = self.org.settings(&self.tenant).await?;
        let workers = self.org.workers(&self.tenant).await?;
        let now = self.now();
        let mut worker = Worker::new(self.tenant.clone(), new.agent.clone(), new.role.trim(), new.department, now);
        worker.reports_to = new.reports_to;
        worker.manager = new.manager;
        worker.autonomy = new.autonomy.unwrap_or(settings.default_level);
        worker.weekly_budget_micros = new.weekly_budget_micros;
        worker.task_time_ms = new.task_time_ms.unwrap_or(worker.task_time_ms);
        worker.still_ask = new.still_ask;
        let mut all = workers;
        if all.iter().any(|w| w.agent == worker.agent) {
            return Err(invalid(format!("{} is already a worker.", worker.agent)));
        }
        all.push(worker.clone());
        validate_org(&settings, &all)?;
        let event = self.draft(
            OrgEventKind::Hired,
            Some(&worker.agent),
            format!("{} was hired as {} in {}, at {}", worker.agent, worker.role, worker.department, worker.autonomy),
            by,
            None,
        );
        self.org.put_worker(&self.tenant, &worker, None, &[event]).await?;
        self.worker(&new.agent).await
    }

    /// Change a worker. Lowering a manager below its reports is refused: lower the reports first.
    pub async fn edit(&self, agent: &AgentSlug, fields: WorkerFields, by: &str) -> Result<Worker> {
        let _lock = self.gate.lock().await;
        let settings = self.org.settings(&self.tenant).await?;
        let stored = self.worker(agent).await?;
        let mut next = stored.clone();
        next.role = fields.role.trim().to_string();
        next.department = fields.department;
        next.reports_to = fields.reports_to;
        next.manager = fields.manager;
        next.autonomy = fields.autonomy;
        next.weekly_budget_micros = fields.weekly_budget_micros;
        next.task_time_ms = fields.task_time_ms;
        next.still_ask = fields.still_ask;
        if fields.schedule_enabled && !stored.schedule.enabled {
            // Turning it on does not run the slots that went by while it was off.
            next.schedule.last_plan = Some(self.now());
            next.schedule.last_report = Some(self.now());
        }
        next.schedule.enabled = fields.schedule_enabled;
        next.updated_at = self.now();
        let mut all = self.org.workers(&self.tenant).await?;
        for w in &mut all {
            if w.agent == next.agent {
                *w = next.clone();
            }
        }
        validate_org(&settings, &all)?;
        let mut events = Vec::new();
        if stored.autonomy != next.autonomy {
            events.push(self.draft(
                OrgEventKind::LevelChanged,
                Some(agent),
                format!("Level changed from {} to {}", stored.autonomy, next.autonomy),
                by,
                None,
            ));
        }
        let mut same_level = WorkerFields::of(&stored);
        same_level.autonomy = next.autonomy;
        if same_level != WorkerFields::of(&next) {
            events.push(self.draft(OrgEventKind::Edited, Some(agent), format!("{agent} was edited"), by, None));
        }
        if !self.org.put_worker(&self.tenant, &next, Some(stored.revision), &events).await? {
            return Err(invalid(format!("{agent} was changed by somebody else just now. Try again.")));
        }
        self.worker(agent).await
    }

    /// Let a worker go. Somebody who has reports, or leads a department, is not let go until they are moved.
    pub async fn fire(&self, agent: &AgentSlug, by: &str) -> Result<()> {
        let _lock = self.gate.lock().await;
        let settings = self.org.settings(&self.tenant).await?;
        let workers = self.org.workers(&self.tenant).await?;
        let reports: Vec<String> =
            workers.iter().filter(|w| w.reports_to.as_ref() == Some(agent)).map(|w| w.agent.to_string()).collect();
        if !reports.is_empty() {
            return Err(invalid(format!(
                "{agent} has reports ({}). Move them to somebody else first.",
                reports.join(", ")
            )));
        }
        if let Some(d) = settings.departments.iter().find(|d| d.lead.as_ref() == Some(agent)) {
            return Err(invalid(format!("{agent} leads {}. Choose another lead first.", d.name)));
        }
        let event = self.draft(OrgEventKind::Fired, Some(agent), format!("{agent} was let go"), by, None);
        if !self.org.remove_worker(&self.tenant, agent, &[event]).await? {
            return Err(Error::Missing(format!("{agent} is not a worker.")));
        }
        Ok(())
    }

    /// Mark everything up to now as read.
    pub async fn mark_seen(&self) -> Result<()> {
        let _lock = self.gate.lock().await;
        let mut settings = self.org.settings(&self.tenant).await?;
        settings.seen_until = Some(self.now());
        self.org.put_settings(&self.tenant, &settings, &[]).await
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Reading

    async fn facts(&self) -> Result<Facts> {
        let now = self.now();
        let week = self.week_of(now);
        let since = Timestamp(week.millis() - 7 * DAY_MS);
        let projects: Vec<Project> =
            self.projects.projects(&self.tenant).await?.into_iter().filter(|p| !p.archived).collect();
        let mut tasks = Vec::new();
        for project in &projects {
            tasks.extend(self.projects.tasks(&self.tenant, project.id).await?);
        }
        let filter = ListFilter { since: Some(since), limit: EXECUTION_WINDOW, ..ListFilter::default() };
        let executions: Vec<Execution> = self
            .executions
            .list(&self.tenant, &filter)
            .await?
            .into_iter()
            .filter(|e| !e.request.metadata.contains_key(EVAL_LABEL))
            .collect();
        // A parent's usage already includes what its children cost, so a worker's own spend leaves them out.
        let mut children: BTreeMap<ExecutionId, u64> = BTreeMap::new();
        for e in &executions {
            if let Some(parent) = e.parent {
                *children.entry(parent).or_default() += e.usage.cost_micros;
            }
        }
        let own_cost = executions
            .iter()
            .map(|e| (e.id, e.usage.cost_micros.saturating_sub(children.get(&e.id).copied().unwrap_or(0))))
            .collect();
        Ok(Facts { now, week, tasks, executions, own_cost })
    }

    async fn eval_score(&self, agent: &AgentSlug) -> Option<f64> {
        let found = self.agents.agent_by_slug(&self.tenant, agent).await.ok()??;
        let runs = self.agents.eval_runs(&self.tenant, found.id, 10).await.ok()?;
        let run = runs.into_iter().find(|r| r.status == "finished" && !r.results.is_empty())?;
        let passed = run.results.iter().filter(|r| r.passed).count();
        Some(passed as f64 / run.results.len() as f64)
    }

    fn own_figures(&self, facts: &Facts, worker: &Worker, eval: Option<f64>) -> Figures {
        let since = Timestamp(facts.now.millis() - 30 * DAY_MS);
        let (done, all) = on_time_counts(&facts.tasks, &worker.agent, since, |at| self.day_text(at));
        Figures {
            tasks: open_for(&facts.tasks, &worker.agent).count() as u32,
            on_time_done: done,
            on_time_all: all,
            eval,
            week_micros: facts.week_spend(&worker.agent),
            waiting: facts.waiting_of(&worker.agent),
        }
    }

    async fn views(&self, facts: &Facts, settings: &OrgSettings, workers: &[Worker]) -> Result<Vec<WorkerView>> {
        let _ = settings;
        let mut own: BTreeMap<AgentSlug, Figures> = BTreeMap::new();
        let mut info: BTreeMap<AgentSlug, (String, String, Avatar, bool)> = BTreeMap::new();
        for w in workers {
            let eval = self.eval_score(&w.agent).await;
            own.insert(w.agent.clone(), self.own_figures(facts, w, eval));
            let agent = self.agents.agent_by_slug(&self.tenant, &w.agent).await?;
            let described = match &agent {
                Some(a) => self
                    .agents
                    .version(&self.tenant, a.id, a.current_version)
                    .await?
                    .map(|v| (v.spec.name, v.spec.description, v.spec.avatar, a.archived)),
                None => None,
            };
            info.insert(
                w.agent.clone(),
                described.unwrap_or_else(|| (w.agent.to_string(), String::new(), Avatar::default(), true)),
            );
        }
        let direct: BTreeMap<AgentSlug, Vec<AgentSlug>> = {
            let mut map: BTreeMap<AgentSlug, Vec<AgentSlug>> = BTreeMap::new();
            for w in workers {
                if let Some(boss) = &w.reports_to {
                    map.entry(boss.clone()).or_default().push(w.agent.clone());
                }
            }
            map
        };
        fn tree(
            agent: &AgentSlug,
            own: &BTreeMap<AgentSlug, Figures>,
            direct: &BTreeMap<AgentSlug, Vec<AgentSlug>>,
            depth: usize,
        ) -> Figures {
            let mut total = own.get(agent).cloned().unwrap_or_default();
            if depth > 20 {
                return total;
            }
            total.eval = None;
            for child in direct.get(agent).into_iter().flatten() {
                total.add(&tree(child, own, direct, depth + 1));
            }
            total
        }
        let mut out = Vec::new();
        for w in workers {
            let reports = direct.get(&w.agent).cloned().unwrap_or_default();
            let own_f = own.get(&w.agent).cloned().unwrap_or_default();
            let figures = if reports.is_empty() { own_f.clone() } else { tree(&w.agent, &own, &direct, 0) };
            let (name, description, avatar, archived) = info.get(&w.agent).cloned().unwrap_or_default();
            out.push(WorkerView {
                budget_left_micros: left(w.weekly_budget_micros, own_f.week_micros),
                status: facts.status_of(&w.agent),
                worker: w.clone(),
                name,
                description,
                avatar,
                figures,
                own: own_f,
                reports,
                archived,
            });
        }
        Ok(out)
    }

    /// The organisation: workers with their figures, and the numbers above the chart.
    pub async fn overview(&self) -> Result<Overview> {
        let facts = self.facts().await?;
        let settings = self.org.settings(&self.tenant).await?;
        let workers = self.org.workers(&self.tenant).await?;
        let views = self.views(&facts, &settings, &workers).await?;
        let week_end_day = self.day_text(Timestamp(facts.week.millis() + 6 * DAY_MS + 12 * 3_600_000));
        let week_day = self.day_text(facts.week);
        let open: Vec<&Task> = facts.tasks.iter().filter(|t| t.state != TaskState::Done).collect();
        let due_this_week = open
            .iter()
            .filter(|t| {
                t.due.as_ref().is_some_and(|d| d.as_str() >= week_day.as_str() && d.as_str() <= week_end_day.as_str())
            })
            .count() as u32;
        let waiting = facts.executions.iter().filter(|e| e.state == ExecutionState::WaitingForApproval).count() as u32;
        let reviews = open.iter().filter(|t| t.state == TaskState::Review).count() as u32;
        let running: Vec<&Execution> = facts
            .executions
            .iter()
            .filter(|e| {
                matches!(
                    e.state,
                    ExecutionState::Queued
                        | ExecutionState::Running
                        | ExecutionState::WaitingForModel
                        | ExecutionState::WaitingForTool
                        | ExecutionState::WaitingForChildren
                )
            })
            .collect();
        let busy: BTreeSet<&AgentSlug> = running.iter().filter_map(|e| e.agent.as_ref()).map(|a| &a.slug).collect();
        let you = YouView {
            to_review: reviews,
            approvals: waiting,
            reports: workers.iter().filter(|w| w.reports_to.is_none()).map(|w| w.agent.clone()).collect(),
        };
        Ok(Overview {
            budget_micros: settings.weekly_budget_micros,
            week_micros: facts.org_spend(),
            settings,
            workers: views,
            you,
            open_tasks: open.len() as u32,
            due_this_week,
            waiting_approvals: waiting,
            waiting_reviews: reviews,
            running: running.len() as u32,
            busy: busy.len() as u32,
            week: week_day,
        })
    }

    /// One worker's page.
    pub async fn detail(&self, agent: &AgentSlug) -> Result<WorkerDetail> {
        let facts = self.facts().await?;
        let settings = self.org.settings(&self.tenant).await?;
        let workers = self.org.workers(&self.tenant).await?;
        if !workers.iter().any(|w| &w.agent == agent) {
            return Err(Error::Missing(format!("{agent} is not a worker.")));
        }
        let view = self
            .views(&facts, &settings, &workers)
            .await?
            .into_iter()
            .find(|v| &v.worker.agent == agent)
            .ok_or_else(|| Error::Missing(format!("{agent} is not a worker.")))?;
        let today = day_start(facts.now, self.zone()).unwrap_or(facts.now);
        let mut spend_days = vec![0u64; SPARK_DAYS];
        for e in facts.executions.iter().filter(|e| e.agent.as_ref().is_some_and(|a| a.slug == *agent)) {
            let age = (today.millis() - e.created_at.millis()).div_euclid(DAY_MS);
            // A run from today has age -1 or 0 depending on the hour; days before are 1.. back.
            let back = if e.created_at >= today { 0 } else { (age + 1) as usize };
            if back < SPARK_DAYS {
                spend_days[SPARK_DAYS - 1 - back] += facts.own_cost.get(&e.id).copied().unwrap_or(0);
            }
        }
        let span = facts.now.millis() - facts.week.millis();
        let previous = Timestamp(facts.week.millis() - 7 * DAY_MS);
        let spend_last_week_micros = facts
            .executions
            .iter()
            .filter(|e| {
                e.created_at >= previous
                    && e.created_at.millis() < previous.millis() + span
                    && e.agent.as_ref().is_some_and(|a| a.slug == *agent)
            })
            .map(|e| facts.own_cost.get(&e.id).copied().unwrap_or(0))
            .sum();
        let today_text = self.day_text(facts.now);
        let mut tasks: Vec<OpenTask> = open_for(&facts.tasks, agent)
            .map(|t| OpenTask {
                key: t.key.to_string(),
                title: t.title.clone(),
                state: t.state,
                due: t.due.clone(),
                late: t.due.as_ref().is_some_and(|d| d.as_str() < today_text.as_str()),
            })
            .collect();
        tasks.sort_by(|a, b| match (&a.due, &b.due) {
            (Some(x), Some(y)) => x.cmp(y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.key.cmp(&b.key),
        });
        let activity = self.activity_of(&facts, Some(agent), 50).await?;
        let reports = self.org.reports(&self.tenant, Some(agent), false, 10).await?;
        let busy_with = reports.iter().find(|r| r.status == ReportStatus::Running).map(|r| r.kind);
        Ok(WorkerDetail { view, spend_days, spend_last_week_micros, tasks, activity, reports, busy_with })
    }

    async fn activity_of(&self, facts: &Facts, worker: Option<&AgentSlug>, limit: usize) -> Result<Vec<ActivityLine>> {
        let mut lines: Vec<ActivityLine> = Vec::new();
        for e in self.org.events(&self.tenant, worker, 200).await? {
            lines.push(org_line(&e));
        }
        // The runs the workforce started: what they did without asking, what they only proposed, what waits for a person.
        let mut runs: Vec<&Execution> = facts
            .executions
            .iter()
            .filter(|e| e.agent.as_ref().is_some_and(|a| a.autonomy.is_some()))
            .filter(|e| worker.is_none_or(|w| e.agent.as_ref().is_some_and(|a| a.slug == *w)))
            .collect();
        runs.sort_by_key(|e| std::cmp::Reverse(e.created_at));
        runs.truncate(60);
        for run in runs {
            let slug = run.agent.as_ref().map(|a| a.slug.to_string()).unwrap_or_default();
            let events = self.executions.events(run.id, 0, 500).await.unwrap_or_default();
            for event in events {
                let p = &event.payload;
                let what = p["summary"].as_str().map(str::to_string).or_else(|| p["tool"].as_str().map(str::to_string));
                let what = what.unwrap_or_else(|| "a call".into());
                match event.kind {
                    EventKind::ToolUnattended => lines.push(ActivityLine {
                        id: format!("un-{}-{}", run.id, event.seq),
                        at: event.at,
                        level: "INFO",
                        source: slug.clone(),
                        text: format!("{what} ran without asking · {}", p["level"].as_str().unwrap_or("")),
                        kind: "unattended",
                        run: Some(run.id),
                    }),
                    EventKind::ToolSuggested => lines.push(ActivityLine {
                        id: format!("su-{}-{}", run.id, event.seq),
                        at: event.at,
                        level: "INFO",
                        source: slug.clone(),
                        text: format!("Proposed: {what} · it only suggests, so you act"),
                        kind: "suggestion",
                        run: Some(run.id),
                    }),
                    EventKind::WaitingApproval if run.state == ExecutionState::WaitingForApproval => {
                        lines.push(ActivityLine {
                            id: format!("wa-{}-{}", run.id, event.seq),
                            at: event.at,
                            level: "WAIT",
                            source: slug.clone(),
                            text: format!("{what} waits for you"),
                            kind: "waiting",
                            run: Some(run.id),
                        })
                    }
                    _ => {}
                }
            }
            if run.state == ExecutionState::Failed {
                let why = run.failure.as_ref().map(|f| f.message.clone()).unwrap_or_default();
                lines.push(ActivityLine {
                    id: format!("fa-{}", run.id),
                    at: run.updated_at,
                    level: "ERR",
                    source: slug.clone(),
                    text: format!("A run failed: {why}"),
                    kind: "failed",
                    run: Some(run.id),
                });
            }
        }
        lines.sort_by_key(|l| std::cmp::Reverse(l.at));
        lines.truncate(limit);
        Ok(lines)
    }

    /// What the workers did, newest first.
    pub async fn activity(&self, limit: usize) -> Result<Vec<ActivityLine>> {
        let facts = self.facts().await?;
        self.activity_of(&facts, None, if limit == 0 { 100 } else { limit }).await
    }

    /// What is worth a person's eye since they last looked: a plan or report, a proposal, an unattended call at the Notify level
    /// and a budget warning.
    pub async fn notices(&self) -> Result<Vec<Notice>> {
        let facts = self.facts().await?;
        let settings = self.org.settings(&self.tenant).await?;
        let seen = settings.seen_until.unwrap_or(Timestamp(0));
        let mut out = Vec::new();
        for line in self.activity_of(&facts, None, 200).await? {
            if line.at <= seen {
                continue;
            }
            let (kind, title) = match line.kind {
                "suggestion" => ("suggestion", format!("{} has a suggestion", line.source)),
                "unattended" if line.text.ends_with("notify") => {
                    ("notice", format!("{} acted without asking", line.source))
                }
                "plan" => ("report", format!("{} planned the week", line.source)),
                "report" => ("report", format!("{} wrote the weekly report", line.source)),
                "budget" => ("notice", format!("{}: a weekly budget", line.source)),
                _ => continue,
            };
            out.push(Notice {
                id: line.id,
                kind,
                at: line.at,
                worker: (line.source != "org").then(|| line.source.clone()),
                title,
                text: line.text,
                run: line.run,
            });
        }
        Ok(out)
    }

    /// The plans and reports, newest first.
    pub async fn reports(&self, worker: Option<&AgentSlug>, limit: usize) -> Result<Vec<OrgReport>> {
        self.org.reports(&self.tenant, worker, false, limit).await
    }

    // -----------------------------------------------------------------------------------------------------------------
    // A manager's week

    fn managed(&self, projects: &[Project], manager: &AgentSlug) -> Vec<String> {
        projects.iter().filter(|p| p.manager.as_ref() == Some(manager)).map(|p| p.key.to_string()).collect()
    }

    async fn start_week_run(
        &self,
        agent: &AgentSlug,
        kind: ReportKind,
        ask: Option<&str>,
        by: &str,
        slot: Option<Timestamp>,
    ) -> Result<OrgReport> {
        let worker = self.worker(agent).await?;
        if !worker.manager {
            return Err(invalid(format!("{agent} is not a manager. Edit it to make it one.")));
        }
        self.live_agent_for_run(agent).await?;
        let now = self.now();
        let week = self.week_of(slot.unwrap_or(now));
        let week_day = self.day_text(week);
        let projects: Vec<Project> =
            self.projects.projects(&self.tenant).await?.into_iter().filter(|p| !p.archived).collect();
        let keys = self.managed(&projects, agent);
        let (prompt, kind_name) = match kind {
            ReportKind::Plan => (plan_prompt(&worker, &week_day, &keys, ask), "plan"),
            ReportKind::Report => (report_prompt(&worker, &week_day, &keys, ask), "report"),
        };
        let mut access = vec!["org:team".to_string()];
        for key in &keys {
            access.push(match kind {
                ReportKind::Plan => format!("project:{key}:write"),
                ReportKind::Report => format!("project:{key}"),
            });
        }
        let stamp = slot.unwrap_or(now).millis();
        let start = RunStart {
            target: Target::Agent { agent: agent.clone(), version: None },
            prompt,
            input: None,
            model: self.model.lock().clone(),
            max_cost_micros: None,
            key: format!("org-{kind_name}-{agent}-{stamp}"),
            metadata: BTreeMap::from([
                ("org".to_string(), kind_name.to_string()),
                ("week".to_string(), week_day.clone()),
            ]),
            access,
            max_active_ms: None,
            autonomy: None,
        };
        let start = self.shape(start).await.map_err(invalid)?;
        let run = self.launcher.start(start).await.map_err(invalid)?;
        let report = OrgReport {
            id: OrgReportId::new(),
            tenant: self.tenant.clone(),
            worker: agent.clone(),
            kind,
            week: Date::new(week_day).map_err(invalid)?,
            run,
            status: ReportStatus::Running,
            body: String::new(),
            error: None,
            by: by.into(),
            started_at: now,
            finished_at: None,
        };
        self.org.add_report(&self.tenant, &report).await?;
        let (event_kind, text) = match kind {
            ReportKind::Plan => (OrgEventKind::PlanStarted, format!("{agent} started planning the week")),
            ReportKind::Report => (OrgEventKind::ReportStarted, format!("{agent} started the weekly report")),
        };
        self.org.add_events(&self.tenant, &[self.draft(event_kind, Some(agent), text, by, Some(run))]).await?;
        Ok(report)
    }

    async fn live_agent_for_run(&self, slug: &AgentSlug) -> Result<()> {
        let agent = self
            .agents
            .agent_by_slug(&self.tenant, slug)
            .await?
            .ok_or_else(|| Error::Missing(format!("There is no agent called {slug}.")))?;
        if agent.archived {
            return Err(invalid(format!("{slug} is archived.")));
        }
        Ok(())
    }

    /// Ask a manager to plan the week. The run follows its level and budget like any other of the workforce.
    pub async fn plan_week(&self, agent: &AgentSlug, ask: Option<&str>) -> Result<OrgReport> {
        let _lock = self.gate.lock().await;
        self.start_week_run(agent, ReportKind::Plan, ask, "you", None).await
    }

    /// Ask a manager for the weekly report.
    pub async fn write_report(&self, agent: &AgentSlug, ask: Option<&str>) -> Result<OrgReport> {
        let _lock = self.gate.lock().await;
        self.start_week_run(agent, ReportKind::Report, ask, "you", None).await
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Keeping up

    /// Read the plans and reports that were going, run the schedules that are due and record the budget notices.
    pub async fn reconcile(&self) -> Result<()> {
        let _lock = self.gate.lock().await;
        self.finish_reports().await?;
        self.run_schedules().await?;
        self.budget_notices().await
    }

    async fn finish_reports(&self) -> Result<()> {
        for report in self.org.reports(&self.tenant, None, true, 50).await? {
            let outcome = match self.launcher.outcome(report.run).await {
                Ok(outcome) => outcome,
                Err(_) => continue,
            };
            let mut done = report.clone();
            let (event_kind, text) = match outcome {
                Outcome::Running { .. } | Outcome::WaitingForPerson { .. } => continue,
                Outcome::Completed { text, .. } => {
                    done.status = ReportStatus::Done;
                    done.body = text;
                    match report.kind {
                        ReportKind::Plan => (OrgEventKind::PlanDone, format!("{} planned the week", report.worker)),
                        ReportKind::Report => {
                            (OrgEventKind::ReportDone, format!("{} wrote the weekly report", report.worker))
                        }
                    }
                }
                Outcome::Failed { reason, .. } => {
                    done.status = ReportStatus::Failed;
                    done.error = Some(reason.clone());
                    (
                        OrgEventKind::ReportFailed,
                        format!("{}: the {} failed: {reason}", report.worker, kind_word(report.kind)),
                    )
                }
                Outcome::Cancelled => {
                    done.status = ReportStatus::Failed;
                    done.error = Some("It was cancelled.".into());
                    (
                        OrgEventKind::ReportFailed,
                        format!("{}: the {} was cancelled", report.worker, kind_word(report.kind)),
                    )
                }
            };
            done.finished_at = Some(self.now());
            let event = self.draft(event_kind, Some(&report.worker), text, &report.by, Some(report.run));
            self.org.update_report(&self.tenant, &done, &[event]).await?;
        }
        Ok(())
    }

    async fn run_schedules(&self) -> Result<()> {
        let now = self.now();
        for worker in self.org.workers(&self.tenant).await? {
            if !worker.manager || !worker.schedule.enabled {
                continue;
            }
            let mut next = worker.clone();
            let mut events = Vec::new();
            for (kind, when, last) in [
                (ReportKind::Plan, &worker.schedule.plan, worker.schedule.last_plan),
                (ReportKind::Report, &worker.schedule.report, worker.schedule.last_report),
            ] {
                let from = last.unwrap_or(now);
                let fires = fires_between(when, worker.schedule.timezone.as_deref().or(self.zone()), from, now, 20)
                    .unwrap_or_default();
                let Some(slot) = fires.last().copied() else {
                    if last.is_none() {
                        match kind {
                            ReportKind::Plan => next.schedule.last_plan = Some(now),
                            ReportKind::Report => next.schedule.last_report = Some(now),
                        }
                    }
                    continue;
                };
                match kind {
                    ReportKind::Plan => next.schedule.last_plan = Some(now),
                    ReportKind::Report => next.schedule.last_report = Some(now),
                }
                let missed = fires.len() - usize::from(now.millis() - slot.millis() <= MISS_GRACE_MS);
                if missed > 0 {
                    events.push(self.draft(
                        OrgEventKind::ScheduleMissed,
                        Some(&worker.agent),
                        format!(
                            "{} missed {missed} scheduled {} while ZORIX was closed",
                            worker.agent,
                            kind_word(kind)
                        ),
                        "schedule",
                        None,
                    ));
                }
                if now.millis() - slot.millis() <= MISS_GRACE_MS
                    && let Err(error) = self.start_week_run(&worker.agent, kind, None, "schedule", Some(slot)).await
                {
                    events.push(self.draft(
                        OrgEventKind::ReportFailed,
                        Some(&worker.agent),
                        format!("{}: the scheduled {} could not start: {error}", worker.agent, kind_word(kind)),
                        "schedule",
                        None,
                    ));
                }
            }
            if next.schedule != worker.schedule || !events.is_empty() {
                // Re-read: starting a run wrote events, not the worker, so the revision is still the one read.
                let _ = self.org.put_worker(&self.tenant, &next, Some(worker.revision), &events).await?;
            }
        }
        Ok(())
    }

    async fn budget_notices(&self) -> Result<()> {
        let workers = self.org.workers(&self.tenant).await?;
        let settings = self.org.settings(&self.tenant).await?;
        if workers.iter().all(|w| w.weekly_budget_micros.is_none()) && settings.weekly_budget_micros.is_none() {
            return Ok(());
        }
        let facts = self.facts().await?;
        let earlier = self.org.events(&self.tenant, None, 500).await?;
        let this_week = |kind: OrgEventKind, worker: Option<&AgentSlug>| {
            earlier.iter().any(|e| e.kind == kind && e.worker.as_ref() == worker && e.at >= facts.week)
        };
        let mut drafts = Vec::new();
        let mut check = |who: Option<&AgentSlug>, budget: Option<u64>, spent: u64, label: String| {
            let Some(budget) = budget else { return };
            if spent >= budget && !this_week(OrgEventKind::BudgetSpent, who) {
                drafts.push(self.draft(
                    OrgEventKind::BudgetSpent,
                    who,
                    format!("{label} spent its weekly budget ({} of {})", usd(spent), usd(budget)),
                    "system",
                    None,
                ));
            } else if spent < budget && is_near(budget, spent) && !this_week(OrgEventKind::BudgetNear, who) {
                drafts.push(self.draft(
                    OrgEventKind::BudgetNear,
                    who,
                    format!(
                        "{label} used {}% of its weekly budget ({} of {})",
                        spent * 100 / budget,
                        usd(spent),
                        usd(budget)
                    ),
                    "system",
                    None,
                ));
            }
        };
        for w in &workers {
            check(Some(&w.agent), w.weekly_budget_micros, facts.week_spend(&w.agent), w.agent.to_string());
        }
        check(None, settings.weekly_budget_micros, facts.org_spend(), "The organisation".into());
        if !drafts.is_empty() {
            self.org.add_events(&self.tenant, &drafts).await?;
        }
        Ok(())
    }

    // -----------------------------------------------------------------------------------------------------------------
    // What a manager's tool shows

    /// Who reports to `agent` (below it too), with their open tasks, spend and level: what `org.team` returns.
    pub async fn team_of(&self, agent: &AgentSlug) -> Result<Value> {
        let facts = self.facts().await?;
        let workers = self.org.workers(&self.tenant).await?;
        let settings = self.org.settings(&self.tenant).await?;
        let mut below: Vec<&Worker> = Vec::new();
        let mut frontier = vec![agent.clone()];
        while let Some(boss) = frontier.pop() {
            for w in workers.iter().filter(|w| w.reports_to.as_ref() == Some(&boss)) {
                if !below.iter().any(|b| b.agent == w.agent) && w.agent != *agent {
                    below.push(w);
                    frontier.push(w.agent.clone());
                }
            }
        }
        let today = self.day_text(facts.now);
        let team: Vec<Value> = below
            .iter()
            .map(|w| {
                let spent = facts.week_spend(&w.agent);
                let open: Vec<Value> = open_for(&facts.tasks, &w.agent)
                    .map(|t| {
                        json!({
                            "key": t.key.to_string(), "title": t.title, "state": t.state.as_str(),
                            "due": t.due, "late": t.due.as_ref().is_some_and(|d| d.as_str() < today.as_str()),
                        })
                    })
                    .collect();
                json!({
                    "agent": w.agent.to_string(), "role": w.role, "department": w.department,
                    "reports_to": w.reports_to.as_ref().map(ToString::to_string).unwrap_or_else(|| "you".into()),
                    "level": w.autonomy.as_str(),
                    "task_time_minutes": w.task_time_ms / 60_000,
                    "weekly_budget_usd": w.weekly_budget_micros.map(|m| m as f64 / 1_000_000.0),
                    "spent_this_week_usd": spent as f64 / 1_000_000.0,
                    "left_this_week_usd": left(w.weekly_budget_micros, spent).map(|m| m as f64 / 1_000_000.0),
                    "open_tasks": open,
                })
            })
            .collect();
        Ok(json!({
            "manager": agent.to_string(),
            "week_starts": self.day_text(facts.week),
            "organisation_budget_usd": settings.weekly_budget_micros.map(|m| m as f64 / 1_000_000.0),
            "organisation_spent_usd": facts.org_spend() as f64 / 1_000_000.0,
            "team": team,
        }))
    }
}

fn kind_word(kind: ReportKind) -> &'static str {
    match kind {
        ReportKind::Plan => "plan",
        ReportKind::Report => "report",
    }
}

fn usd(micros: u64) -> String {
    format!("${:.2}", micros as f64 / 1_000_000.0)
}

fn org_line(event: &OrgEvent) -> ActivityLine {
    let (level, kind) = match event.kind {
        OrgEventKind::Hired
        | OrgEventKind::Edited
        | OrgEventKind::LevelChanged
        | OrgEventKind::Fired
        | OrgEventKind::SettingsChanged => ("INFO", "org"),
        OrgEventKind::PlanStarted | OrgEventKind::ReportStarted => ("INFO", "org"),
        OrgEventKind::PlanDone => ("OK", "plan"),
        OrgEventKind::ReportDone => ("OK", "report"),
        OrgEventKind::ReportFailed => ("ERR", "failed"),
        OrgEventKind::BudgetNear | OrgEventKind::BudgetSpent | OrgEventKind::ScheduleMissed => ("WARN", "budget"),
    };
    ActivityLine {
        id: format!("org-{}", event.seq),
        at: event.at,
        level,
        source: event.worker.as_ref().map(ToString::to_string).unwrap_or_else(|| "org".into()),
        text: event.text.clone(),
        kind,
        run: event.run,
    }
}

#[async_trait]
impl RunShaper for WorkforceService {
    async fn shape(&self, mut start: RunStart) -> std::result::Result<RunStart, String> {
        let slug = match &start.target {
            Target::Agent { agent, .. } => agent.clone(),
            Target::Team { team } => match self.agents.team(&self.tenant, team).await.map_err(|e| e.to_string())? {
                Some(team) => team.lead,
                None => return Ok(start),
            },
        };
        let Some(worker) = self.org.worker(&self.tenant, &slug).await.map_err(|e| e.to_string())? else {
            return Ok(start);
        };
        let settings = self.org.settings(&self.tenant).await.map_err(|e| e.to_string())?;
        let facts = self.facts().await.map_err(|e| e.to_string())?;
        let Shaped { autonomy, max_active_ms, max_cost_micros, note } = shape_run(
            &worker,
            &settings,
            facts.week_spend(&slug),
            facts.org_spend(),
            start.max_cost_micros,
            start.max_active_ms,
        );
        start.metadata.insert("worker".into(), slug.to_string());
        start.metadata.insert("autonomy".into(), autonomy.level.as_str().into());
        if let Some(note) = note {
            start.metadata.insert("autonomy_note".into(), note);
        }
        start.max_cost_micros = max_cost_micros;
        start.max_active_ms = Some(max_active_ms);
        start.autonomy = Some(autonomy);
        Ok(start)
    }
}
