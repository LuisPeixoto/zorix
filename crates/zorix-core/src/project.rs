//! Projects and tasks (ADR-0036, ADR-0037): the records, their validation and the store contract.
//!
//! A task with an agent or a team makes ordinary executions; this module is only the model. Running tasks lives in
//! `zorix-projects`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::agent::AgentSlug;
use crate::error::{Error, Result};
use crate::ids::{ExecutionId, ProjectId, TaskId, TenantId};
use crate::time::Timestamp;

/// Most projects one tenant keeps.
pub const MAX_PROJECTS: usize = 100;
/// Most tasks in one project.
pub const MAX_PROJECT_TASKS: usize = 2_000;
/// Most tasks a task may depend on.
pub const MAX_DEPENDENCIES: usize = 20;
/// Most links on a task.
pub const MAX_LINKS: usize = 20;
/// Most deliverables on a task.
pub const MAX_DELIVERABLES: usize = 100;
/// Most runs a task keeps in its record (the newest).
pub const MAX_TASK_RUNS: usize = 50;

const MAX_TITLE: usize = 200;
const MAX_TEXT: usize = 20_000;
const MAX_URL: usize = 2_000;

fn invalid(what: impl Into<String>) -> Error {
    Error::InvalidInput(what.into())
}

// ---------------------------------------------------------------------------------------------------------------------
// Keys and dates

/// A project's key: 2 to 5 capital letters or digits, starting with a letter (`OPS`). It starts the keys of its tasks.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ProjectKey(String);

impl ProjectKey {
    /// Check and wrap a key.
    pub fn new(value: impl Into<String>) -> std::result::Result<Self, String> {
        let value = value.into();
        let mut chars = value.chars();
        let ok = (2..=5).contains(&value.len())
            && chars.next().is_some_and(|c| c.is_ascii_uppercase())
            && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
        if ok {
            Ok(Self(value))
        } else {
            Err(format!(
                "“{value}” is not a valid project key: use 2 to 5 capital letters or digits, starting with a letter."
            ))
        }
    }

    /// The key as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProjectKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for ProjectKey {
    type Err = String;

    fn from_str(text: &str) -> std::result::Result<Self, String> {
        Self::new(text)
    }
}

impl TryFrom<String> for ProjectKey {
    type Error = String;

    fn try_from(value: String) -> std::result::Result<Self, String> {
        Self::new(value)
    }
}

impl From<ProjectKey> for String {
    fn from(key: ProjectKey) -> String {
        key.0
    }
}

/// A task's key: its project's key and a number that is never reused (`OPS-38`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TaskKey {
    /// The project's key.
    pub project: ProjectKey,
    /// The number within the project, from 1.
    pub number: u32,
}

impl TaskKey {
    /// A key from its parts. The number starts at 1.
    pub fn new(project: ProjectKey, number: u32) -> std::result::Result<Self, String> {
        if number == 0 {
            return Err("A task number starts at 1.".into());
        }
        Ok(Self { project, number })
    }
}

impl fmt::Display for TaskKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.project, self.number)
    }
}

impl FromStr for TaskKey {
    type Err = String;

    fn from_str(text: &str) -> std::result::Result<Self, String> {
        let bad =
            || format!("“{text}” is not a task key: write the project key, a hyphen and a number, such as OPS-38.");
        let (project, number) = text.rsplit_once('-').ok_or_else(bad)?;
        let number: u32 = number.parse().map_err(|_| bad())?;
        Self::new(ProjectKey::new(project).map_err(|_| bad())?, number).map_err(|_| bad())
    }
}

impl TryFrom<String> for TaskKey {
    type Error = String;

    fn try_from(value: String) -> std::result::Result<Self, String> {
        value.parse()
    }
}

impl From<TaskKey> for String {
    fn from(key: TaskKey) -> String {
        key.to_string()
    }
}

/// A calendar day with no time zone, written `YYYY-MM-DD`. Due dates are days, so they mean the same everywhere.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Date(String);

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => 29,
        _ => 28,
    }
}

impl Date {
    /// Check and wrap a day.
    pub fn new(value: impl Into<String>) -> std::result::Result<Self, String> {
        let value = value.into();
        let bad = || format!("“{value}” is not a date: write it as YYYY-MM-DD.");
        let bytes = value.as_bytes();
        if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
            return Err(bad());
        }
        let part = |range: std::ops::Range<usize>| value[range].parse::<u32>().ok();
        let (year, month, day) =
            (part(0..4).ok_or_else(bad)?, part(5..7).ok_or_else(bad)?, part(8..10).ok_or_else(bad)?);
        if !(1970..=2200).contains(&year) || !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month)
        {
            return Err(bad());
        }
        Ok(Self(value))
    }

    /// The day as `YYYY-MM-DD`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for Date {
    type Err = String;

    fn from_str(text: &str) -> std::result::Result<Self, String> {
        Self::new(text)
    }
}

impl TryFrom<String> for Date {
    type Error = String;

    fn try_from(value: String) -> std::result::Result<Self, String> {
        Self::new(value)
    }
}

impl From<Date> for String {
    fn from(date: Date) -> String {
        date.0
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Who, states, priorities

/// Who has a task, or who did something: you (the person using ZORIX), an agent or a team. Other people wait for Phase 10.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Who {
    /// The person using ZORIX.
    You,
    /// An agent.
    Agent {
        /// Its slug.
        slug: AgentSlug,
    },
    /// A team, through its lead.
    Team {
        /// Its slug.
        slug: AgentSlug,
    },
}

impl Who {
    /// Whether work given to this assignee starts a run.
    pub fn runs(&self) -> bool {
        !matches!(self, Who::You)
    }

    /// A short readable name: `you`, or the slug.
    pub fn label(&self) -> String {
        match self {
            Who::You => "you".into(),
            Who::Agent { slug } | Who::Team { slug } => slug.to_string(),
        }
    }
}

/// The four lanes of a project.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    /// Not started.
    Backlog,
    /// Being worked on. For an agent or a team this is when its run starts.
    InProgress,
    /// Waiting for a person to accept it.
    Review,
    /// Finished.
    Done,
}

impl TaskState {
    /// All lanes, in board order.
    pub const ALL: [TaskState; 4] = [TaskState::Backlog, TaskState::InProgress, TaskState::Review, TaskState::Done];

    /// The word the store and the command line use.
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskState::Backlog => "backlog",
            TaskState::InProgress => "in_progress",
            TaskState::Review => "review",
            TaskState::Done => "done",
        }
    }
}

impl FromStr for TaskState {
    type Err = String;

    fn from_str(text: &str) -> std::result::Result<Self, String> {
        match text.to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
            "backlog" => Ok(TaskState::Backlog),
            "in_progress" | "progress" | "doing" => Ok(TaskState::InProgress),
            "review" => Ok(TaskState::Review),
            "done" => Ok(TaskState::Done),
            _ => Err(format!("“{text}” is not a state: use backlog, in-progress, review or done.")),
        }
    }
}

/// How urgent a task is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskPriority {
    /// Drop other things.
    Urgent,
    /// Soon.
    High,
    /// The default.
    #[default]
    Normal,
    /// When there is time.
    Low,
}

impl FromStr for TaskPriority {
    type Err = String;

    fn from_str(text: &str) -> std::result::Result<Self, String> {
        match text.to_ascii_lowercase().as_str() {
            "urgent" => Ok(TaskPriority::Urgent),
            "high" => Ok(TaskPriority::High),
            "normal" => Ok(TaskPriority::Normal),
            "low" => Ok(TaskPriority::Low),
            _ => Err(format!("“{text}” is not a priority: use urgent, high, normal or low.")),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Task parts

/// A link on a task: a pull request, an issue, a document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskLink {
    /// The address.
    pub url: String,
    /// A name for it, when the person gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// What kind of thing a deliverable is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliverableKind {
    /// Text, usually the final answer of a run (Markdown).
    Answer,
    /// An address.
    Link,
    /// A file on this computer.
    File,
}

/// Something a task produced.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deliverable {
    /// Identifies it within the task.
    pub id: String,
    /// What it is.
    pub kind: DeliverableKind,
    /// A short name.
    pub title: String,
    /// The text, the address or the path.
    pub body: String,
    /// Who made it (none for ZORIX itself).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Who>,
    /// When.
    pub at: Timestamp,
    /// The run that made it, when one did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<ExecutionId>,
}

/// Where one run of a task stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    /// Going.
    Running,
    /// Stopped for a person.
    Waiting,
    /// Ended well.
    Finished,
    /// Ended badly.
    Failed,
    /// Stopped on purpose.
    Cancelled,
}

impl RunOutcome {
    /// Whether the run is over.
    pub fn ended(&self) -> bool {
        matches!(self, RunOutcome::Finished | RunOutcome::Failed | RunOutcome::Cancelled)
    }
}

/// One run a task started.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRun {
    /// The execution.
    pub execution: ExecutionId,
    /// When it started.
    pub started_at: Timestamp,
    /// When it ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<Timestamp>,
    /// Where it stands.
    pub outcome: RunOutcome,
    /// The note the person gave when sending the task back, which this run was asked to address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Why it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// What it cost, in millionths of a dollar, when it ended.
    #[serde(default)]
    pub cost_micros: u64,
}

/// A run a task is waiting to start: set when an agent's or a team's task moves to In progress (or is run again), cleared when the
/// run starts. Whatever keeps tasks moving starts it as soon as the task is not blocked.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRequest {
    /// What the person said when sending the task back, which the run must address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Why the run could not be started, when it could not. The request then waits for a person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What a person must do about a task. Never stored: it is read from the task.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NeedsYou {
    /// The task is in Review.
    Review,
    /// The last run failed.
    RunFailed,
    /// The run stopped for a person.
    RunWaiting,
}

/// One task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    /// Its id.
    pub id: TaskId,
    /// The owner.
    pub tenant: TenantId,
    /// Its project.
    pub project: ProjectId,
    /// Its key (`OPS-38`).
    pub key: TaskKey,
    /// A short name.
    pub title: String,
    /// What to do, in Markdown.
    #[serde(default)]
    pub description: String,
    /// The lane.
    pub state: TaskState,
    /// How urgent.
    #[serde(default)]
    pub priority: TaskPriority,
    /// Who has it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<Who>,
    /// 0 to 100.
    #[serde(default)]
    pub progress: u8,
    /// The first day of work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<Date>,
    /// The last day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<Date>,
    /// The tasks of the same project that must be Done first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<TaskKey>,
    /// Links.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<TaskLink>,
    /// What it produced.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deliverables: Vec<Deliverable>,
    /// The runs it started, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runs: Vec<TaskRun>,
    /// A run waiting to start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_requested: Option<RunRequest>,
    /// Place in the lane: lower is higher up.
    #[serde(default)]
    pub rank: i64,
    /// Goes up with every write, for optimistic writes. The store sets it.
    #[serde(default)]
    pub revision: u64,
    /// When it was made.
    pub created_at: Timestamp,
    /// When it last changed.
    pub updated_at: Timestamp,
    /// When it went to Done, if it is there. Older tasks have none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_at: Option<Timestamp>,
}

impl Task {
    /// A new task in the Backlog, with nothing else set. The store fills in the revision.
    pub fn new(tenant: TenantId, project: ProjectId, key: TaskKey, title: impl Into<String>, now: Timestamp) -> Self {
        Self {
            id: TaskId::new(),
            tenant,
            project,
            key,
            title: title.into(),
            description: String::new(),
            state: TaskState::Backlog,
            priority: TaskPriority::Normal,
            assignee: None,
            progress: 0,
            start: None,
            due: None,
            depends_on: Vec::new(),
            links: Vec::new(),
            deliverables: Vec::new(),
            runs: Vec::new(),
            run_requested: None,
            rank: 0,
            revision: 0,
            created_at: now,
            updated_at: now,
            done_at: None,
        }
    }

    /// Check the task on its own. Dependencies are checked against the project's other tasks by [`check_dependencies`].
    pub fn validate(&self) -> Result<()> {
        let title = self.title.trim();
        if title.is_empty() {
            return Err(invalid("A task needs a title."));
        }
        if title.chars().count() > MAX_TITLE {
            return Err(invalid(format!("A task title is at most {MAX_TITLE} characters.")));
        }
        if self.description.chars().count() > MAX_TEXT {
            return Err(invalid(format!("A task description is at most {MAX_TEXT} characters.")));
        }
        if self.progress > 100 {
            return Err(invalid("Progress is from 0 to 100."));
        }
        if let (Some(start), Some(due)) = (&self.start, &self.due)
            && start > due
        {
            return Err(invalid("A task cannot start after it is due."));
        }
        if self.depends_on.len() > MAX_DEPENDENCIES {
            return Err(invalid(format!("A task depends on at most {MAX_DEPENDENCIES} others.")));
        }
        let mut seen = BTreeSet::new();
        for key in &self.depends_on {
            if key == &self.key {
                return Err(invalid("A task cannot depend on itself."));
            }
            if key.project != self.key.project {
                return Err(invalid(format!(
                    "{key} is in another project: a task can only depend on tasks of its own project."
                )));
            }
            if !seen.insert(key) {
                return Err(invalid(format!("{key} is listed twice.")));
            }
        }
        if self.links.len() > MAX_LINKS {
            return Err(invalid(format!("A task has at most {MAX_LINKS} links.")));
        }
        for link in &self.links {
            let url = link.url.trim();
            if url.is_empty() || url.len() > MAX_URL || !(url.starts_with("https://") || url.starts_with("http://")) {
                return Err(invalid("A link is an http or https address."));
            }
        }
        if self.deliverables.len() > MAX_DELIVERABLES {
            return Err(invalid(format!("A task has at most {MAX_DELIVERABLES} deliverables.")));
        }
        Ok(())
    }

    /// The tasks it waits for: dependencies among `siblings` that are not Done.
    pub fn blocked_by(&self, siblings: &[Task]) -> Vec<TaskKey> {
        self.depends_on
            .iter()
            .filter(|key| siblings.iter().find(|t| &t.key == *key).is_none_or(|t| t.state != TaskState::Done))
            .cloned()
            .collect()
    }

    /// The run that is going now, when there is one.
    pub fn current_run(&self) -> Option<&TaskRun> {
        self.runs.last().filter(|run| !run.outcome.ended())
    }

    /// What a person must do about it, when anything.
    pub fn needs_you(&self) -> Option<NeedsYou> {
        if self.state == TaskState::Done {
            return None;
        }
        if self.state == TaskState::InProgress && self.run_requested.as_ref().is_some_and(|r| r.error.is_some()) {
            return Some(NeedsYou::RunFailed);
        }
        match self.runs.last().map(|run| run.outcome) {
            Some(RunOutcome::Failed) if self.state == TaskState::InProgress => return Some(NeedsYou::RunFailed),
            Some(RunOutcome::Waiting) => return Some(NeedsYou::RunWaiting),
            _ => {}
        }
        (self.state == TaskState::Review).then_some(NeedsYou::Review)
    }

    /// The total cost of its runs, in millionths of a dollar.
    pub fn cost_micros(&self) -> u64 {
        self.runs.iter().map(|run| run.cost_micros).sum()
    }
}

/// Check that `deps` for the task `key` name tasks of `siblings` (its project) and make no cycle.
pub fn check_dependencies(key: &TaskKey, deps: &[TaskKey], siblings: &[Task]) -> Result<()> {
    let known: BTreeMap<&TaskKey, &Task> = siblings.iter().map(|t| (&t.key, t)).collect();
    for dep in deps {
        if !known.contains_key(dep) {
            return Err(invalid(format!("{dep} does not exist.")));
        }
    }
    // A cycle exists when walking the dependencies of any dependency comes back to `key`.
    let mut stack: Vec<&TaskKey> = deps.iter().collect();
    let mut visited: BTreeSet<&TaskKey> = BTreeSet::new();
    while let Some(next) = stack.pop() {
        if next == key {
            return Err(invalid(format!(
                "That would make {key} wait for itself through {}.",
                deps.first().map(ToString::to_string).unwrap_or_default()
            )));
        }
        if !visited.insert(next) {
            continue;
        }
        if let Some(task) = known.get(next) {
            stack.extend(task.depends_on.iter());
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------------
// Projects

/// One project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    /// Its id.
    pub id: ProjectId,
    /// The owner.
    pub tenant: TenantId,
    /// Its key, which never changes.
    pub key: ProjectKey,
    /// What people call it.
    pub name: String,
    /// What it is for.
    #[serde(default)]
    pub description: String,
    /// The agent that plans it, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manager: Option<AgentSlug>,
    /// The first day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<Date>,
    /// The last day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<Date>,
    /// The most one task run may cost, in millionths of a dollar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_limit_micros: Option<u64>,
    /// Hidden from the list of active projects.
    #[serde(default)]
    pub archived: bool,
    /// The number the next task gets. Never goes down, so keys are not reused. The store keeps it.
    #[serde(default = "first_number")]
    pub next_task: u32,
    /// When it was made.
    pub created_at: Timestamp,
    /// When it last changed.
    pub updated_at: Timestamp,
}

fn first_number() -> u32 {
    1
}

impl Project {
    /// A new project with nothing else set.
    pub fn new(tenant: TenantId, key: ProjectKey, name: impl Into<String>, now: Timestamp) -> Self {
        Self {
            id: ProjectId::new(),
            tenant,
            key,
            name: name.into(),
            description: String::new(),
            manager: None,
            start: None,
            due: None,
            cost_limit_micros: None,
            archived: false,
            next_task: 1,
            created_at: now,
            updated_at: now,
        }
    }

    /// Check the project.
    pub fn validate(&self) -> Result<()> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(invalid("A project needs a name."));
        }
        if name.chars().count() > MAX_TITLE {
            return Err(invalid(format!("A project name is at most {MAX_TITLE} characters.")));
        }
        if self.description.chars().count() > MAX_TEXT {
            return Err(invalid(format!("A project description is at most {MAX_TEXT} characters.")));
        }
        if let (Some(start), Some(due)) = (&self.start, &self.due)
            && start > due
        {
            return Err(invalid("A project cannot start after it is due."));
        }
        if self.cost_limit_micros == Some(0) {
            return Err(invalid("A cost limit of zero would stop every task run. Leave it out for no limit."));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// History

/// What happened.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProjectEventKind {
    /// The project was made.
    ProjectCreated,
    /// The project's details changed.
    ProjectEdited,
    /// The project was archived or brought back.
    ProjectArchived {
        /// Whether it is archived now.
        archived: bool,
    },
    /// A task was made.
    TaskCreated {
        /// Its title.
        title: String,
    },
    /// A task's details changed.
    TaskEdited,
    /// A task moved from one lane to another.
    Moved {
        /// The lane it left.
        from: TaskState,
        /// The lane it entered.
        to: TaskState,
        /// The person who approved the move when it needed approval.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        approved_by: Option<String>,
        /// What the person said when sending a task back.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    /// A task was given to someone, or to nobody.
    Assigned {
        /// The new assignee.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<Who>,
    },
    /// A run began.
    RunStarted {
        /// The execution.
        execution: ExecutionId,
    },
    /// A run ended well.
    RunFinished {
        /// The execution.
        execution: ExecutionId,
    },
    /// A run ended badly.
    RunFailed {
        /// The execution.
        execution: ExecutionId,
        /// Why.
        reason: String,
    },
    /// A run could not be started (the agent is missing, or the model refused).
    StartFailed {
        /// Why.
        reason: String,
    },
    /// A run was stopped on purpose.
    RunCancelled {
        /// The execution.
        execution: ExecutionId,
    },
    /// A task is waiting for others.
    Blocked {
        /// The tasks it waits for.
        by: Vec<TaskKey>,
    },
    /// A task reported progress.
    Progress {
        /// 0 to 100.
        percent: u8,
    },
    /// A deliverable was added.
    Deliverable {
        /// Its title.
        title: String,
    },
    /// A comment.
    Comment {
        /// The text.
        text: String,
    },
    /// A link was added.
    Linked {
        /// The address.
        url: String,
    },
    /// The manager was asked to plan.
    ManagerAsked {
        /// The execution.
        execution: ExecutionId,
    },
}

/// An event before the store numbers it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectEventDraft {
    /// The task it concerns, when it concerns one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<TaskKey>,
    /// Who did it (none for ZORIX itself, such as a run finishing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Who>,
    /// What happened.
    pub kind: ProjectEventKind,
    /// When.
    pub at: Timestamp,
}

/// One line of a project's history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectEvent {
    /// Its place in the project's history, from 1.
    pub seq: u64,
    /// The project.
    pub project: ProjectId,
    /// The task it concerns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<TaskKey>,
    /// Who did it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Who>,
    /// What happened.
    pub kind: ProjectEventKind,
    /// When.
    pub at: Timestamp,
}

/// How many tasks a project has in one lane.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskCount {
    /// The project.
    pub project: ProjectId,
    /// The lane.
    pub state: TaskState,
    /// How many.
    pub count: u32,
}

// ---------------------------------------------------------------------------------------------------------------------
// The store

/// Where projects, tasks and their history are kept. Every call takes the tenant; nothing crosses tenants.
#[async_trait]
pub trait ProjectStore: Send + Sync {
    /// Every project of a tenant, archived ones too, by key.
    async fn projects(&self, tenant: &TenantId) -> Result<Vec<Project>>;
    /// One project.
    async fn project(&self, tenant: &TenantId, id: ProjectId) -> Result<Option<Project>>;
    /// One project by key.
    async fn project_by_key(&self, tenant: &TenantId, key: &ProjectKey) -> Result<Option<Project>>;
    /// Make a project. Fails when the key is taken or the tenant has [`MAX_PROJECTS`]. Writes the `ProjectCreated` event.
    async fn create_project(&self, tenant: &TenantId, project: Project, by: Option<Who>) -> Result<Project>;
    /// Replace a project's details. Its key, its task counter and its creation time are kept as stored.
    async fn update_project(
        &self,
        tenant: &TenantId,
        project: &Project,
        events: &[ProjectEventDraft],
    ) -> Result<Project>;
    /// Delete a project with its tasks and history.
    async fn remove_project(&self, tenant: &TenantId, id: ProjectId) -> Result<bool>;

    /// Take the next task number of a project. A number is never given twice, even when the task is not saved.
    async fn next_task_number(&self, tenant: &TenantId, project: ProjectId) -> Result<u32>;
    /// Write a task and its events together. A new task passes `expected` as none; an existing one is written only when
    /// `expected` is its stored revision, which then goes up by one. Returns false when somebody else changed it first.
    async fn put_task(
        &self,
        tenant: &TenantId,
        task: &Task,
        expected: Option<u64>,
        events: &[ProjectEventDraft],
    ) -> Result<bool>;
    /// One task.
    async fn task(&self, tenant: &TenantId, key: &TaskKey) -> Result<Option<Task>>;
    /// The tasks of a project, by rank.
    async fn tasks(&self, tenant: &TenantId, project: ProjectId) -> Result<Vec<Task>>;
    /// The tasks of active projects that are in a lane.
    async fn tasks_in_state(&self, tenant: &TenantId, state: TaskState) -> Result<Vec<Task>>;
    /// Delete a task. The tasks that depended on it stop waiting for it.
    async fn remove_task(&self, tenant: &TenantId, key: &TaskKey) -> Result<bool>;
    /// How many tasks every project has in each lane.
    async fn task_counts(&self, tenant: &TenantId) -> Result<Vec<TaskCount>>;

    /// Add events that belong to no task write (the manager was asked, the project was edited).
    async fn add_events(&self, tenant: &TenantId, project: ProjectId, events: &[ProjectEventDraft]) -> Result<()>;
    /// A project's history, newest first, of one task or of all.
    async fn events(
        &self,
        tenant: &TenantId,
        project: ProjectId,
        task: Option<&TaskKey>,
        limit: usize,
    ) -> Result<Vec<ProjectEvent>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str) -> TaskKey {
        text.parse().unwrap()
    }

    fn task(k: &str, deps: &[&str], state: TaskState) -> Task {
        let mut t = Task::new(TenantId::local(), ProjectId::new(), key(k), format!("Task {k}"), Timestamp(1));
        t.depends_on = deps.iter().map(|d| key(d)).collect();
        t.state = state;
        t
    }

    #[test]
    fn project_keys_are_two_to_five_capitals_or_digits_starting_with_a_letter() {
        for good in ["OPS", "WEB2", "AB", "ABCDE"] {
            assert!(ProjectKey::new(good).is_ok(), "{good}");
        }
        for bad in ["", "A", "ops", "ABCDEF", "1AB", "A-B", "OP S"] {
            assert!(ProjectKey::new(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn task_keys_parse_and_print_the_same_way() {
        let k = key("OPS-38");
        assert_eq!((k.project.as_str(), k.number), ("OPS", 38));
        assert_eq!(k.to_string(), "OPS-38");
        assert!("OPS-0".parse::<TaskKey>().is_err());
        assert!("OPS".parse::<TaskKey>().is_err());
        assert!("ops-1".parse::<TaskKey>().is_err());
        assert_eq!(serde_json::to_string(&k).unwrap(), "\"OPS-38\"");
        assert_eq!(serde_json::from_str::<TaskKey>("\"WEB-7\"").unwrap(), key("WEB-7"));
    }

    #[test]
    fn dates_are_real_calendar_days() {
        assert!(Date::new("2026-02-28").is_ok());
        assert!(Date::new("2028-02-29").is_ok());
        assert!(Date::new("2026-02-29").is_err());
        assert!(Date::new("2026-13-01").is_err());
        assert!(Date::new("2026-4-1").is_err());
        assert!(Date::new("2026-12-20").unwrap() > Date::new("2026-11-30").unwrap());
    }

    #[test]
    fn a_task_is_checked_on_its_own() {
        let mut t = task("OPS-1", &[], TaskState::Backlog);
        assert!(t.validate().is_ok());
        t.title = "  ".into();
        assert!(t.validate().is_err());
        t.title = "ok".into();
        t.progress = 101;
        assert!(t.validate().is_err());
        t.progress = 50;
        t.start = Some(Date::new("2026-12-02").unwrap());
        t.due = Some(Date::new("2026-12-01").unwrap());
        assert!(t.validate().unwrap_err().to_string().contains("start after"));
        t.start = None;
        t.depends_on = vec![key("OPS-1")];
        assert!(t.validate().unwrap_err().to_string().contains("itself"));
        t.depends_on = vec![key("WEB-2")];
        assert!(t.validate().unwrap_err().to_string().contains("another project"));
        t.depends_on = vec![];
        t.links = vec![TaskLink { url: "file:///etc/passwd".into(), title: None }];
        assert!(t.validate().is_err());
    }

    #[test]
    fn dependencies_must_exist_and_may_not_make_a_cycle() {
        let a = task("OPS-1", &[], TaskState::Backlog);
        let b = task("OPS-2", &["OPS-1"], TaskState::Backlog);
        let c = task("OPS-3", &["OPS-2"], TaskState::Backlog);
        let all = vec![a, b, c];
        assert!(check_dependencies(&key("OPS-3"), &[key("OPS-1")], &all).is_ok());
        assert!(
            check_dependencies(&key("OPS-1"), &[key("OPS-3")], &all).is_err(),
            "1 would wait for 3, which waits for 2, which waits for 1"
        );
        assert!(
            check_dependencies(&key("OPS-1"), &[key("OPS-9")], &all)
                .unwrap_err()
                .to_string()
                .contains("does not exist")
        );
    }

    #[test]
    fn a_task_waits_for_dependencies_that_are_not_done() {
        let done = task("OPS-1", &[], TaskState::Done);
        let open = task("OPS-2", &[], TaskState::Review);
        let t = task("OPS-3", &["OPS-1", "OPS-2"], TaskState::Backlog);
        assert_eq!(t.blocked_by(&[done, open]), vec![key("OPS-2")]);
    }

    fn run(outcome: RunOutcome) -> TaskRun {
        TaskRun {
            execution: ExecutionId::new(),
            started_at: Timestamp(1),
            ended_at: outcome.ended().then_some(Timestamp(2)),
            outcome,
            note: None,
            reason: None,
            cost_micros: 10,
        }
    }

    #[test]
    fn needs_you_is_read_from_the_task() {
        let mut t = task("OPS-1", &[], TaskState::Backlog);
        assert_eq!(t.needs_you(), None);
        t.state = TaskState::Review;
        assert_eq!(t.needs_you(), Some(NeedsYou::Review));
        t.state = TaskState::InProgress;
        t.runs = vec![run(RunOutcome::Failed)];
        assert_eq!(t.needs_you(), Some(NeedsYou::RunFailed));
        t.runs = vec![run(RunOutcome::Waiting)];
        assert_eq!(t.needs_you(), Some(NeedsYou::RunWaiting));
        t.runs = vec![run(RunOutcome::Running)];
        assert_eq!(t.needs_you(), None);
        assert!(t.current_run().is_some());
        t.state = TaskState::Done;
        t.runs = vec![run(RunOutcome::Failed)];
        assert_eq!(t.needs_you(), None, "a finished task needs nobody");
        assert_eq!(t.cost_micros(), 10);
    }

    #[test]
    fn who_and_states_round_trip_through_json_and_text() {
        let who = Who::Agent { slug: AgentSlug::new("atlas").unwrap() };
        let json = serde_json::to_string(&who).unwrap();
        assert_eq!(json, r#"{"type":"agent","slug":"atlas"}"#);
        assert_eq!(serde_json::from_str::<Who>(&json).unwrap(), who);
        assert!(who.runs() && !Who::You.runs());
        assert_eq!("in-progress".parse::<TaskState>().unwrap(), TaskState::InProgress);
        assert_eq!(serde_json::to_string(&TaskState::InProgress).unwrap(), "\"in_progress\"");
        assert_eq!("URGENT".parse::<TaskPriority>().unwrap(), TaskPriority::Urgent);
        assert!("later".parse::<TaskPriority>().is_err());
    }

    #[test]
    fn a_project_is_checked() {
        let mut p = Project::new(TenantId::local(), ProjectKey::new("OPS").unwrap(), "Q4 Operations", Timestamp(1));
        assert!(p.validate().is_ok());
        p.name = " ".into();
        assert!(p.validate().is_err());
        p.name = "Ok".into();
        p.cost_limit_micros = Some(0);
        assert!(p.validate().is_err());
    }
}
