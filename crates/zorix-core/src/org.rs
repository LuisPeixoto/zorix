//! The workforce: workers over agents, departments, who reports to whom, and the store contract (ADR-0038).
//!
//! A worker is a record over an existing agent; it never changes the agent or its versions. Every chain of `reports_to` ends at you
//! (`None`), and a worker can only be as free as, and spend as much as, the one it reports to.

use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::agent::AgentSlug;
use crate::autonomy::AutonomyLevel;
use crate::error::{Error, Result};
use crate::ids::{ExecutionId, OrgReportId, TenantId};
use crate::project::Date;
use crate::time::Timestamp;
use crate::workflow::{Day, When};

/// Most workers one tenant keeps.
pub const MAX_WORKERS: usize = 100;
/// Most departments.
pub const MAX_DEPARTMENTS: usize = 20;
/// The shortest time a task may be given: one minute.
pub const MIN_TASK_TIME_MS: u64 = 60_000;
/// The longest: one day.
pub const MAX_TASK_TIME_MS: u64 = 24 * 60 * 60 * 1_000;
/// Most entries in a worker's own always-ask list.
pub const MAX_STILL_ASK: usize = 20;
/// The default time for a task: two hours.
pub const DEFAULT_TASK_TIME_MS: u64 = 2 * 60 * 60 * 1_000;

fn invalid(what: impl Into<String>) -> Error {
    Error::InvalidInput(what.into())
}

/// A grouping of workers for people to read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Department {
    /// Its short name (never changes).
    pub slug: String,
    /// What people call it.
    pub name: String,
    /// One sentence about what it is for.
    #[serde(default)]
    pub description: String,
    /// Its lead: a worker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lead: Option<AgentSlug>,
}

/// When a manager plans and reports on its own. Off until a person turns it on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkerSchedule {
    /// Whether it runs at all.
    #[serde(default)]
    pub enabled: bool,
    /// When it plans the week.
    pub plan: When,
    /// When it writes the report.
    pub report: When,
    /// The time zone (an IANA name); none is this computer's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    /// The last time the plan slot was acted on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_plan: Option<Timestamp>,
    /// The last time the report slot was acted on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_report: Option<Timestamp>,
}

impl Default for WorkerSchedule {
    /// Plan on Mondays at 09:00 and report on Fridays at 16:00, off.
    fn default() -> Self {
        Self {
            enabled: false,
            plan: When::Daily { times: vec!["09:00".into()], days: vec![Day::Mon] },
            report: When::Daily { times: vec!["16:00".into()], days: vec![Day::Fri] },
            timezone: None,
            last_plan: None,
            last_report: None,
        }
    }
}

/// An agent with a place in the organisation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Worker {
    /// The owner.
    pub tenant: TenantId,
    /// The agent it is a record over (one worker per agent).
    pub agent: AgentSlug,
    /// What it does there: "Engineering lead".
    pub role: String,
    /// Its department's slug.
    pub department: String,
    /// Who it answers to: another worker, or you (none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reports_to: Option<AgentSlug>,
    /// A virtual manager: it plans the week and writes the report.
    #[serde(default)]
    pub manager: bool,
    /// How much it may do without asking.
    pub autonomy: AutonomyLevel,
    /// The most it may spend in a week, in millionths of a dollar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekly_budget_micros: Option<u64>,
    /// The most active time a task may take, in milliseconds.
    pub task_time_ms: u64,
    /// Access patterns whose calls always ask, whatever the level.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub still_ask: Vec<String>,
    /// Its plan and report schedule (used by managers).
    #[serde(default)]
    pub schedule: WorkerSchedule,
    /// Goes up with every write, for optimistic writes. The store sets it.
    #[serde(default)]
    pub revision: u64,
    /// When it was hired.
    pub hired_at: Timestamp,
    /// When it last changed.
    pub updated_at: Timestamp,
}

impl Worker {
    /// A new worker at the Ask level with the default time for a task.
    pub fn new(
        tenant: TenantId,
        agent: AgentSlug,
        role: impl Into<String>,
        department: impl Into<String>,
        now: Timestamp,
    ) -> Self {
        Self {
            tenant,
            agent,
            role: role.into(),
            department: department.into(),
            reports_to: None,
            manager: false,
            autonomy: AutonomyLevel::Ask,
            weekly_budget_micros: None,
            task_time_ms: DEFAULT_TASK_TIME_MS,
            still_ask: Vec::new(),
            schedule: WorkerSchedule::default(),
            revision: 0,
            hired_at: now,
            updated_at: now,
        }
    }

    /// Check what a person typed, apart from how the worker fits the organisation (see [`validate_org`]).
    pub fn validate(&self) -> Result<()> {
        let role = self.role.trim();
        if role.is_empty() || role.chars().count() > 60 {
            return Err(invalid("The role has 1 to 60 characters."));
        }
        if !(MIN_TASK_TIME_MS..=MAX_TASK_TIME_MS).contains(&self.task_time_ms) {
            return Err(invalid("The time for a task is between 1 minute and 24 hours."));
        }
        if self.weekly_budget_micros == Some(0) {
            return Err(invalid(
                "A weekly budget of nothing means the worker cannot run; leave it empty for no limit.",
            ));
        }
        if self.still_ask.len() > MAX_STILL_ASK {
            return Err(invalid(format!("The list of what always asks has {MAX_STILL_ASK} entries at most.")));
        }
        crate::access::compile_access(&self.still_ask).map_err(invalid)?;
        if self.reports_to.as_ref() == Some(&self.agent) {
            return Err(invalid("A worker cannot report to itself."));
        }
        Ok(())
    }
}

/// What applies to the whole organisation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrgSettings {
    /// The most all workers may spend in a week, in millionths of a dollar. None means no limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekly_budget_micros: Option<u64>,
    /// The level new workers start at.
    pub default_level: AutonomyLevel,
    /// The departments.
    #[serde(default)]
    pub departments: Vec<Department>,
    /// Notices (reports, suggestions, unattended calls, budget warnings) from before this time are read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen_until: Option<Timestamp>,
}

impl Default for OrgSettings {
    /// No limit, new workers ask first, and the three departments of a small organisation.
    fn default() -> Self {
        let dept = |slug: &str, name: &str| Department {
            slug: slug.into(),
            name: name.into(),
            description: String::new(),
            lead: None,
        };
        Self {
            weekly_budget_micros: None,
            default_level: AutonomyLevel::Ask,
            departments: vec![
                dept("operations", "Operations"),
                dept("research", "Research"),
                dept("engineering", "Engineering"),
            ],
            seen_until: None,
        }
    }
}

impl OrgSettings {
    /// Check what a person typed.
    pub fn validate(&self) -> Result<()> {
        if self.weekly_budget_micros == Some(0) {
            return Err(invalid("A weekly budget of nothing stops every worker; leave it empty for no limit."));
        }
        if self.departments.len() > MAX_DEPARTMENTS {
            return Err(invalid(format!("There are {MAX_DEPARTMENTS} departments at most.")));
        }
        let mut seen = BTreeSet::new();
        for d in &self.departments {
            if AgentSlug::new(d.slug.clone()).is_err() {
                return Err(invalid(format!(
                    "“{}” is not a department's short name: lower case, digits and hyphens.",
                    d.slug
                )));
            }
            if d.name.trim().is_empty() || d.name.chars().count() > 60 {
                return Err(invalid("A department's name has 1 to 60 characters."));
            }
            if d.description.chars().count() > 300 {
                return Err(invalid("A department's description has 300 characters at most."));
            }
            if !seen.insert(d.slug.as_str()) {
                return Err(invalid(format!("There are two departments called {}.", d.slug)));
            }
        }
        Ok(())
    }
}

/// Check the whole organisation: every worker is valid, belongs to a department, reports to somebody who exists, no chain loops,
/// and nobody is freer or richer than the one it reports to (ADR-0038).
pub fn validate_org(settings: &OrgSettings, workers: &[Worker]) -> Result<()> {
    settings.validate()?;
    if workers.len() > MAX_WORKERS {
        return Err(invalid(format!("There are {MAX_WORKERS} workers at most.")));
    }
    let by_agent: BTreeMap<&AgentSlug, &Worker> = workers.iter().map(|w| (&w.agent, w)).collect();
    if by_agent.len() != workers.len() {
        return Err(invalid("An agent is one worker at most."));
    }
    for w in workers {
        w.validate()?;
        if !settings.departments.iter().any(|d| d.slug == w.department) {
            return Err(invalid(format!("{} is in the department “{}”, which does not exist.", w.agent, w.department)));
        }
        if let Some(boss) = &w.reports_to {
            let Some(manager) = by_agent.get(boss) else {
                return Err(invalid(format!("{} reports to {boss}, who is not a worker.", w.agent)));
            };
            if w.autonomy > manager.autonomy {
                return Err(invalid(format!(
                    "{} is at {}, which is freer than {boss}, who it reports to (at {}). A worker cannot be freer than its manager.",
                    w.agent, w.autonomy, manager.autonomy
                )));
            }
            if let Some(cap) = manager.weekly_budget_micros {
                match w.weekly_budget_micros {
                    Some(own) if own <= cap => {}
                    Some(_) => {
                        return Err(invalid(format!(
                            "{} has a larger weekly budget than {boss}, who it reports to. A worker's budget cannot be larger than its manager's.",
                            w.agent
                        )));
                    }
                    None => {
                        return Err(invalid(format!(
                            "{boss} has a weekly budget, so {} needs one too, no larger than it.",
                            w.agent
                        )));
                    }
                }
            }
        }
        // Follow the chain up: it must end at you without meeting itself.
        let mut at = w;
        let mut steps = 0;
        while let Some(next) = at.reports_to.as_ref().and_then(|b| by_agent.get(b)) {
            steps += 1;
            if std::ptr::eq(*next, w) || steps > workers.len() {
                return Err(invalid(format!("{} would answer to itself through its chain of managers.", w.agent)));
            }
            at = next;
        }
    }
    for d in &settings.departments {
        if let Some(lead) = &d.lead
            && !by_agent.contains_key(lead)
        {
            return Err(invalid(format!("The lead of {} is {lead}, who is not a worker.", d.name)));
        }
    }
    Ok(())
}

/// What a manager's run wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportKind {
    /// The week's plan.
    Plan,
    /// The report at the end of the week.
    Report,
}

/// How a plan or report is going.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportStatus {
    /// The run is going.
    Running,
    /// It finished and the answer is kept.
    Done,
    /// The run failed or was cancelled.
    Failed,
}

/// A weekly plan or report by a virtual manager.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrgReport {
    /// Its id.
    pub id: OrgReportId,
    /// The owner.
    pub tenant: TenantId,
    /// The manager.
    pub worker: AgentSlug,
    /// Plan or report.
    pub kind: ReportKind,
    /// The Monday of its week.
    pub week: Date,
    /// The run that wrote it.
    pub run: ExecutionId,
    /// How it is going.
    pub status: ReportStatus,
    /// The answer, once it is done.
    #[serde(default)]
    pub body: String,
    /// Why it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// How it started: `you` or `schedule`.
    pub by: String,
    /// When it started.
    pub started_at: Timestamp,
    /// When it ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<Timestamp>,
}

/// What happened in the organisation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrgEventKind {
    /// An agent was hired.
    Hired,
    /// A worker was edited.
    Edited,
    /// A worker's level changed.
    LevelChanged,
    /// A worker was let go.
    Fired,
    /// The settings or the departments changed.
    SettingsChanged,
    /// A manager started planning the week.
    PlanStarted,
    /// A manager wrote the plan.
    PlanDone,
    /// A manager started the report.
    ReportStarted,
    /// A manager wrote the report.
    ReportDone,
    /// A plan or report failed.
    ReportFailed,
    /// A worker used most of its week's budget.
    BudgetNear,
    /// A worker, or the organisation, spent its week's budget.
    BudgetSpent,
    /// A scheduled slot was missed (the app was closed).
    ScheduleMissed,
}

/// An event before the store numbers it.
#[derive(Clone, Debug, PartialEq)]
pub struct OrgEventDraft {
    /// What happened.
    pub kind: OrgEventKind,
    /// The worker it is about, when there is one.
    pub worker: Option<AgentSlug>,
    /// One sentence.
    pub text: String,
    /// Who did it: `you`, `schedule` or `system`.
    pub by: String,
    /// The run it came from.
    pub run: Option<ExecutionId>,
    /// When.
    pub at: Timestamp,
}

/// A stored event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrgEvent {
    /// Position in the order: strictly increasing.
    pub seq: u64,
    /// What happened.
    pub kind: OrgEventKind,
    /// The worker it is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker: Option<AgentSlug>,
    /// One sentence.
    pub text: String,
    /// Who did it.
    pub by: String,
    /// The run it came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<ExecutionId>,
    /// When.
    pub at: Timestamp,
}

/// Stores the workforce.
#[async_trait]
pub trait OrgStore: Send + Sync {
    /// The settings; the defaults when none were saved.
    async fn settings(&self, tenant: &TenantId) -> Result<OrgSettings>;
    /// Save the settings.
    async fn put_settings(&self, tenant: &TenantId, settings: &OrgSettings, events: &[OrgEventDraft]) -> Result<()>;

    /// Every worker, by hiring time.
    async fn workers(&self, tenant: &TenantId) -> Result<Vec<Worker>>;
    /// One worker.
    async fn worker(&self, tenant: &TenantId, agent: &AgentSlug) -> Result<Option<Worker>>;
    /// Write a worker and its events together. A new worker passes `expected` as none and fails when the agent is already one; an
    /// existing one is written only when `expected` is its stored revision, which then goes up by one. Returns false when
    /// somebody else changed it first.
    async fn put_worker(
        &self,
        tenant: &TenantId,
        worker: &Worker,
        expected: Option<u64>,
        events: &[OrgEventDraft],
    ) -> Result<bool>;
    /// Let a worker go. Returns false when it was not one.
    async fn remove_worker(&self, tenant: &TenantId, agent: &AgentSlug, events: &[OrgEventDraft]) -> Result<bool>;

    /// Keep a plan or report that just started.
    async fn add_report(&self, tenant: &TenantId, report: &OrgReport) -> Result<()>;
    /// Replace a report (its run finished).
    async fn update_report(&self, tenant: &TenantId, report: &OrgReport, events: &[OrgEventDraft]) -> Result<()>;
    /// One report.
    async fn report(&self, tenant: &TenantId, id: OrgReportId) -> Result<Option<OrgReport>>;
    /// Reports, newest first, of one worker or of all, and optionally only those still running.
    async fn reports(
        &self,
        tenant: &TenantId,
        worker: Option<&AgentSlug>,
        running_only: bool,
        limit: usize,
    ) -> Result<Vec<OrgReport>>;

    /// Add events that belong to no write above.
    async fn add_events(&self, tenant: &TenantId, events: &[OrgEventDraft]) -> Result<()>;
    /// The organisation's history, newest first, of one worker or of all.
    async fn events(&self, tenant: &TenantId, worker: Option<&AgentSlug>, limit: usize) -> Result<Vec<OrgEvent>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slug(s: &str) -> AgentSlug {
        AgentSlug::new(s).unwrap()
    }

    fn worker(agent: &str, dept: &str, boss: Option<&str>, level: AutonomyLevel, budget: Option<u64>) -> Worker {
        let mut w = Worker::new(TenantId::local(), slug(agent), "Role", dept, Timestamp(1));
        w.reports_to = boss.map(slug);
        w.autonomy = level;
        w.weekly_budget_micros = budget;
        w
    }

    fn chain() -> Vec<Worker> {
        vec![
            worker("atlas", "operations", None, AutonomyLevel::Budgeted, Some(60_000_000)),
            worker("forge", "engineering", Some("atlas"), AutonomyLevel::Budgeted, Some(25_000_000)),
            worker("reviewer", "engineering", Some("forge"), AutonomyLevel::Notify, Some(10_000_000)),
        ]
    }

    fn check(workers: &[Worker]) -> Result<()> {
        validate_org(&OrgSettings::default(), workers)
    }

    fn message(result: Result<()>) -> String {
        result.unwrap_err().to_string()
    }

    #[test]
    fn a_chain_that_shrinks_and_ends_at_you_is_valid() {
        check(&chain()).unwrap();
    }

    #[test]
    fn a_worker_cannot_be_freer_than_the_one_it_reports_to() {
        let mut w = chain();
        w[2].autonomy = AutonomyLevel::Full;
        assert!(message(check(&w)).contains("freer than forge"));
    }

    #[test]
    fn a_budget_cannot_be_larger_than_the_managers_or_missing_under_one_that_has_it() {
        let mut w = chain();
        w[2].weekly_budget_micros = Some(30_000_000);
        assert!(message(check(&w)).contains("larger weekly budget than forge"));
        let mut w = chain();
        w[2].weekly_budget_micros = None;
        assert!(message(check(&w)).contains("needs one too"));
        // Under a manager with no limit, a worker may have any.
        let mut w = chain();
        w[0].weekly_budget_micros = None;
        check(&w).unwrap();
    }

    #[test]
    fn a_chain_that_loops_is_refused() {
        let w = vec![
            worker("atlas", "operations", Some("forge"), AutonomyLevel::Ask, None),
            worker("forge", "engineering", Some("atlas"), AutonomyLevel::Ask, None),
        ];
        assert!(message(check(&w)).contains("answer to itself"));
        let alone = vec![worker("atlas", "operations", Some("atlas"), AutonomyLevel::Ask, None)];
        assert!(message(check(&alone)).contains("cannot report to itself"));
    }

    #[test]
    fn a_worker_reports_to_a_worker_and_belongs_to_a_department_that_exist() {
        let w = vec![worker("atlas", "operations", Some("ghost"), AutonomyLevel::Ask, None)];
        assert!(message(check(&w)).contains("ghost, who is not a worker"));
        let w = vec![worker("atlas", "sales", None, AutonomyLevel::Ask, None)];
        assert!(message(check(&w)).contains("department “sales”, which does not exist"));
    }

    #[test]
    fn an_agent_is_one_worker_at_most() {
        let w = vec![
            worker("atlas", "operations", None, AutonomyLevel::Ask, None),
            worker("atlas", "research", None, AutonomyLevel::Ask, None),
        ];
        assert!(message(check(&w)).contains("one worker at most"));
    }

    #[test]
    fn what_a_person_typed_is_checked() {
        let mut w = worker("atlas", "operations", None, AutonomyLevel::Ask, None);
        w.role = "  ".into();
        assert!(w.validate().is_err());
        let mut w = worker("atlas", "operations", None, AutonomyLevel::Ask, Some(0));
        assert!(w.validate().unwrap_err().to_string().contains("empty for no limit"));
        w.weekly_budget_micros = None;
        w.task_time_ms = 1_000;
        assert!(w.validate().is_err());
        w.task_time_ms = DEFAULT_TASK_TIME_MS;
        w.still_ask = vec!["not an access entry".into()];
        assert!(w.validate().is_err());
        w.still_ask = vec!["repo:acme/web:write".into()];
        w.validate().unwrap();
    }

    #[test]
    fn departments_are_unique_and_a_lead_is_a_worker() {
        let mut settings = OrgSettings::default();
        settings.departments.push(settings.departments[0].clone());
        assert!(message(validate_org(&settings, &[])).contains("two departments"));
        let mut settings = OrgSettings::default();
        settings.departments[0].lead = Some(slug("nobody"));
        assert!(message(validate_org(&settings, &[])).contains("not a worker"));
        settings.departments[0].lead = Some(slug("atlas"));
        validate_org(&settings, &chain()).unwrap();
    }

    #[test]
    fn the_schedule_is_off_by_default_with_mondays_and_fridays() {
        let schedule = WorkerSchedule::default();
        assert!(!schedule.enabled);
        assert_eq!(schedule.plan.describe(), "Mondays 09:00");
        assert_eq!(schedule.report.describe(), "Fridays 16:00");
    }
}
