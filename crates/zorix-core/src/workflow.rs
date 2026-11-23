//! Workflows (ADR-0033 to ADR-0035): a versioned graph of triggers and steps that creates executions.
//!
//! This module is the model only: the definition and its validation, the records of a run and of a wait, and the store
//! contract. Running a workflow lives in `zorix-workflows`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::str::FromStr;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::AgentSlug;
use crate::approval::canonical_hash;
use crate::error::{Error, Result};
use crate::ids::{ExecutionId, TenantId, WaitId, WorkflowId, WorkflowRunId};
use crate::time::Timestamp;

/// Most steps (triggers included) in one workflow.
pub const MAX_STEPS: usize = 50;
/// Most workflows one tenant keeps.
pub const MAX_WORKFLOWS: usize = 200;
/// Most published versions of one workflow.
pub const MAX_WORKFLOW_VERSIONS: usize = 500;
/// Most items one loop step runs.
pub const MAX_LOOP_ITEMS: usize = 8;
/// Most steps running at the same time in one run.
pub const MAX_ACTIVE_STEPS: usize = 8;
/// A chain of workflows that start each other stops at this depth.
pub const MAX_CHAIN: u32 = 4;
/// The shortest interval of an "every N minutes" schedule.
pub const MIN_EVERY_MINUTES: u32 = 5;
/// The shortest time-out or wait, in milliseconds.
pub const MIN_TIMEOUT_MS: u64 = 60_000;
/// The longest time-out or wait: thirty days.
pub const MAX_TIMEOUT_MS: u64 = 30 * 24 * 60 * 60 * 1000;

const MAX_TEXT: usize = 20_000;

/// A workflow's short name: lowercase letters, digits and single hyphens, 1 to 48 characters.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct WorkflowSlug(String);

impl WorkflowSlug {
    /// Check and wrap a slug.
    pub fn new(value: impl Into<String>) -> std::result::Result<Self, String> {
        let value = value.into();
        let ok = (1..=48).contains(&value.len())
            && !value.starts_with('-')
            && !value.ends_with('-')
            && !value.contains("--")
            && value.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if ok {
            Ok(Self(value))
        } else {
            Err(format!(
                "“{value}” is not a valid workflow name: use 1 to 48 lowercase letters, digits and single hyphens."
            ))
        }
    }

    /// The slug as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WorkflowSlug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for WorkflowSlug {
    type Err = String;

    fn from_str(text: &str) -> std::result::Result<Self, String> {
        Self::new(text)
    }
}

impl TryFrom<String> for WorkflowSlug {
    type Error = String;

    fn try_from(value: String) -> std::result::Result<Self, String> {
        Self::new(value)
    }
}

impl From<WorkflowSlug> for String {
    fn from(slug: WorkflowSlug) -> String {
        slug.0
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The definition

/// A day of the week.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Day {
    /// Monday.
    Mon,
    /// Tuesday.
    Tue,
    /// Wednesday.
    Wed,
    /// Thursday.
    Thu,
    /// Friday.
    Fri,
    /// Saturday.
    Sat,
    /// Sunday.
    Sun,
}

impl Day {
    /// Monday is 1 and Sunday is 7.
    pub fn number(self) -> u32 {
        self as u32 + 1
    }

    fn short(self) -> &'static str {
        ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][self as usize]
    }

    fn plural(self) -> &'static str {
        ["Mondays", "Tuesdays", "Wednesdays", "Thursdays", "Fridays", "Saturdays", "Sundays"][self as usize]
    }
}

/// When a schedule fires.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum When {
    /// Every so many minutes.
    Every {
        /// The interval, at least [`MIN_EVERY_MINUTES`].
        minutes: u32,
    },
    /// At fixed times of the day, on some days.
    Daily {
        /// `HH:MM` times, 24-hour.
        times: Vec<String>,
        /// The days; none means every day.
        #[serde(default)]
        days: Vec<Day>,
    },
    /// A five-field cron expression.
    Cron {
        /// The expression, such as `0 8 * * 1-5`.
        expr: String,
    },
}

impl When {
    /// What people read: "Weekdays 08:00", "Fridays 16:00", "Every 15 minutes".
    pub fn describe(&self) -> String {
        match self {
            When::Every { minutes } if *minutes % 60 == 0 && *minutes >= 60 => {
                let hours = minutes / 60;
                if hours == 1 { "Every hour".into() } else { format!("Every {hours} hours") }
            }
            When::Every { minutes } => format!("Every {minutes} minutes"),
            When::Cron { expr } => format!("Cron {expr}"),
            When::Daily { times, days } => {
                let at = times.join(", ");
                let set: BTreeSet<Day> = days.iter().copied().collect();
                let weekdays: BTreeSet<Day> = [Day::Mon, Day::Tue, Day::Wed, Day::Thu, Day::Fri].into();
                let who = if set.is_empty() || set.len() == 7 {
                    "Daily".to_string()
                } else if set == weekdays {
                    "Weekdays".to_string()
                } else if set.len() == 1 {
                    set.iter().next().map(|d| d.plural().to_string()).unwrap_or_default()
                } else {
                    set.iter().map(|d| d.short()).collect::<Vec<_>>().join(", ")
                };
                format!("{who} {at}")
            }
        }
    }
}

/// Which event of an agent or team run starts a workflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventOn {
    /// A run ended well.
    Finished,
    /// A run failed.
    Failed,
    /// A run ended either way.
    Any,
}

/// What a person says should happen when a time-out passes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Then {
    /// The step is refused and the run stops there.
    #[default]
    Refuse,
    /// The step counts as approved (or takes its default answer) and the run goes on.
    Continue,
}

/// Whether an action waits for a person before it runs (ADR-0035).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ask {
    /// A person approves each call. The default.
    #[default]
    Always,
    /// The call runs unattended. A person confirms this when publishing.
    Never,
}

/// A chat service a message goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Service {
    /// Slack.
    Slack,
    /// Discord.
    Discord,
}

/// A reminder sent to a chat channel while an approval waits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Remind {
    /// How long after the wait began.
    pub after_ms: u64,
    /// The Slack connection to use.
    pub connection: String,
    /// The channel.
    pub channel: String,
}

/// What a step is, and its settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum StepKind {
    /// Started by a person or the command line.
    Manual,
    /// Starts on a schedule.
    Schedule {
        /// When.
        when: When,
        /// An IANA time zone such as `Europe/Lisbon`; the computer's when absent.
        #[serde(default)]
        timezone: Option<String>,
    },
    /// Starts when a web request arrives.
    Webhook {
        /// Where it comes from, for the label ("GitHub").
        source: String,
        /// A short label for the chip ("issue labelled").
        #[serde(default)]
        label: Option<String>,
        /// A condition over the payload; requests it rejects are ignored.
        #[serde(default)]
        filter: Option<String>,
        /// Whether the request must carry a valid `X-Hub-Signature-256`.
        #[serde(default)]
        signed: bool,
    },
    /// Starts when an agent or team run ends.
    Event {
        /// Which ending.
        on: EventOn,
        /// Only runs of this agent; any when absent.
        #[serde(default)]
        agent: Option<AgentSlug>,
    },
    /// Starts when another workflow ends.
    AfterWorkflow {
        /// The workflow.
        workflow: WorkflowSlug,
        /// Which ending.
        on: EventOn,
    },
    /// Runs an agent.
    Agent {
        /// The agent.
        agent: AgentSlug,
        /// What to ask it; may use `{{…}}` templates.
        prompt: String,
        /// A fixed version; the newest when absent.
        #[serde(default)]
        version: Option<u32>,
        /// A model for this step.
        #[serde(default)]
        model: Option<String>,
        /// A cost limit for this step, in millionths of a dollar.
        #[serde(default)]
        max_cost_micros: Option<u64>,
    },
    /// Runs a team through its lead.
    Team {
        /// The team.
        team: AgentSlug,
        /// What to ask it; may use `{{…}}` templates.
        prompt: String,
        /// A model for this step.
        #[serde(default)]
        model: Option<String>,
        /// A cost limit for this step, in millionths of a dollar.
        #[serde(default)]
        max_cost_micros: Option<u64>,
    },
    /// Chooses a branch: its two edges are labelled `yes` and `no`.
    Condition {
        /// The expression.
        expr: String,
    },
    /// Runs an agent for each item of a list.
    Loop {
        /// A template naming a list, such as `{{steps.find.output.items}}`.
        over: String,
        /// The agent.
        agent: AgentSlug,
        /// What to ask it for each item; `{{item}}` is the item.
        prompt: String,
    },
    /// Waits for a while, or until a time of day.
    Wait {
        /// How long, in milliseconds.
        #[serde(default)]
        for_ms: Option<u64>,
        /// Until the next `HH:MM`.
        #[serde(default)]
        until: Option<String>,
    },
    /// Stops until a person approves.
    Approval {
        /// What is being asked; may use `{{…}}` templates.
        message: String,
        /// The values to show the person, as templates.
        #[serde(default)]
        show: Vec<String>,
        /// How long to wait before `then` applies.
        #[serde(default)]
        timeout_ms: Option<u64>,
        /// What happens at the time-out.
        #[serde(default)]
        then: Then,
        /// A reminder in a chat channel.
        #[serde(default)]
        remind: Option<Remind>,
    },
    /// Stops until a person answers a question.
    Question {
        /// The question; may use `{{…}}` templates.
        question: String,
        /// The answer taken when `then` is `continue` and the time runs out.
        #[serde(default)]
        default: Option<String>,
        /// How long to wait before `then` applies.
        #[serde(default)]
        timeout_ms: Option<u64>,
        /// What happens at the time-out.
        #[serde(default)]
        then: Then,
    },
    /// Sends a chat message.
    Send {
        /// The service.
        service: Service,
        /// The connection to use; the only one of that service when absent.
        #[serde(default)]
        connection: Option<String>,
        /// The channel.
        channel: String,
        /// The message; may use `{{…}}` templates.
        text: String,
        /// Whether a person approves each message.
        #[serde(default)]
        ask: Ask,
    },
    /// Makes an HTTP request.
    Http {
        /// `GET`, `POST`, `PUT`, `PATCH` or `DELETE`.
        method: String,
        /// The address; may use `{{…}}` templates.
        url: String,
        /// The body; may use `{{…}}` templates.
        #[serde(default)]
        body: Option<String>,
        /// Whether a person approves each request.
        #[serde(default)]
        ask: Ask,
    },
}

/// The five kinds of node in the design system, which set a step's colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepClass {
    /// How a workflow starts.
    Trigger,
    /// An agent or a team.
    Agent,
    /// A condition, a loop or a wait.
    Logic,
    /// A person decides or answers.
    Approval,
    /// Something is sent or called.
    Action,
}

impl StepKind {
    /// The node kind of the design system.
    pub fn class(&self) -> StepClass {
        match self {
            StepKind::Manual
            | StepKind::Schedule { .. }
            | StepKind::Webhook { .. }
            | StepKind::Event { .. }
            | StepKind::AfterWorkflow { .. } => StepClass::Trigger,
            StepKind::Agent { .. } | StepKind::Team { .. } | StepKind::Loop { .. } => StepClass::Agent,
            StepKind::Condition { .. } | StepKind::Wait { .. } => StepClass::Logic,
            StepKind::Approval { .. } | StepKind::Question { .. } => StepClass::Approval,
            StepKind::Send { .. } | StepKind::Http { .. } => StepClass::Action,
        }
    }

    /// Whether this step starts a workflow.
    pub fn is_trigger(&self) -> bool {
        self.class() == StepClass::Trigger
    }

    /// The name of the kind as used in files and the interface.
    pub fn name(&self) -> &'static str {
        match self {
            StepKind::Manual => "manual",
            StepKind::Schedule { .. } => "schedule",
            StepKind::Webhook { .. } => "webhook",
            StepKind::Event { .. } => "event",
            StepKind::AfterWorkflow { .. } => "after_workflow",
            StepKind::Agent { .. } => "agent",
            StepKind::Team { .. } => "team",
            StepKind::Condition { .. } => "condition",
            StepKind::Loop { .. } => "loop",
            StepKind::Wait { .. } => "wait",
            StepKind::Approval { .. } => "approval",
            StepKind::Question { .. } => "question",
            StepKind::Send { .. } => "send",
            StepKind::Http { .. } => "http",
        }
    }

    /// For a trigger, the text of its chip: "Weekdays 08:00", "GitHub · issue labelled", "After weekly-report".
    pub fn trigger_label(&self) -> Option<String> {
        Some(match self {
            StepKind::Manual => "Manual".into(),
            StepKind::Schedule { when, .. } => when.describe(),
            StepKind::Webhook { source, label, .. } => match label {
                Some(label) => format!("{source} · {label}"),
                None => format!("{source} · webhook"),
            },
            StepKind::Event { on, agent } => {
                let what = match on {
                    EventOn::Finished => "finishes",
                    EventOn::Failed => "fails",
                    EventOn::Any => "ends",
                };
                match agent {
                    Some(agent) => format!("When {agent} {what}"),
                    None => format!("When a run {what}"),
                }
            }
            StepKind::AfterWorkflow { workflow, .. } => format!("After {workflow}"),
            _ => return None,
        })
    }

    /// Whether the step sends something without asking: an action with `ask: never`, or an approval that reminds in a chat channel.
    pub fn unattended(&self) -> bool {
        matches!(
            self,
            StepKind::Send { ask: Ask::Never, .. }
                | StepKind::Http { ask: Ask::Never, .. }
                | StepKind::Approval { remind: Some(_), .. }
        )
    }
}

/// One node of the graph.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// Its name inside the workflow: lowercase letters, digits, `-` and `_`, 1 to 32 characters.
    pub id: String,
    /// What the step does, in words ("Post to #incidents").
    pub title: String,
    /// Where it sits on the canvas.
    #[serde(default)]
    pub x: i32,
    /// Where it sits on the canvas.
    #[serde(default)]
    pub y: i32,
    /// What it is.
    pub kind: StepKind,
}

impl Step {
    /// The SHA-256 of the step's settings (not its place on the canvas), used to confirm an unattended action.
    pub fn hash(&self) -> String {
        let value = serde_json::json!({ "id": self.id, "title": self.title, "kind": self.kind });
        canonical_hash(&value)
    }
}

/// An arrow from one step to the next.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edge {
    /// The step it leaves.
    pub from: String,
    /// The step it enters.
    pub to: String,
    /// `yes` or `no` on the edges of a condition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// The definition of a workflow.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowSpec {
    /// What people call it.
    pub name: String,
    /// One sentence about what it is for.
    #[serde(default)]
    pub description: String,
    /// A group such as "Operations", shown under the name.
    #[serde(default)]
    pub group: String,
    /// The steps, triggers included.
    pub steps: Vec<Step>,
    /// The arrows.
    #[serde(default)]
    pub edges: Vec<Edge>,
    /// A cost limit for one run, in millionths of a dollar.
    #[serde(default)]
    pub budget_micros: Option<u64>,
    /// Minutes of a person's time one run saves, for the "time saved" figure.
    #[serde(default)]
    pub saves_minutes: Option<u32>,
}

fn valid_step_id(id: &str) -> bool {
    (1..=32).contains(&id.len())
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// `HH:MM`, 24-hour.
pub fn valid_time_of_day(text: &str) -> bool {
    let Some((h, m)) = text.split_once(':') else { return false };
    h.len() == 2 && m.len() == 2 && h.parse::<u32>().is_ok_and(|h| h < 24) && m.parse::<u32>().is_ok_and(|m| m < 60)
}

impl WorkflowSpec {
    /// A workflow with the one trigger a person starts by hand and nothing else, for a new draft.
    pub fn blank(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            group: String::new(),
            steps: vec![Step { id: "start".into(), title: "Manual".into(), x: 96, y: 28, kind: StepKind::Manual }],
            edges: Vec::new(),
            budget_micros: None,
            saves_minutes: None,
        }
    }

    /// The SHA-256 of the spec in canonical JSON. Where steps sit on the canvas does not count: moving a node is not a change.
    pub fn hash(&self) -> String {
        let mut value = serde_json::to_value(self).unwrap_or(Value::Null);
        if let Some(steps) = value.get_mut("steps").and_then(Value::as_array_mut) {
            for step in steps {
                if let Some(object) = step.as_object_mut() {
                    object.remove("x");
                    object.remove("y");
                }
            }
        }
        canonical_hash(&value)
    }

    /// A step by id.
    pub fn step(&self, id: &str) -> Option<&Step> {
        self.steps.iter().find(|s| s.id == id)
    }

    /// The trigger steps.
    pub fn triggers(&self) -> impl Iterator<Item = &Step> {
        self.steps.iter().filter(|s| s.kind.is_trigger())
    }

    /// The steps that run an action unattended, which a person must confirm when publishing.
    pub fn unattended(&self) -> Vec<&Step> {
        self.steps.iter().filter(|s| s.kind.unattended()).collect()
    }

    /// The ids of the steps that lead into `id`.
    pub fn inputs(&self, id: &str) -> Vec<&Edge> {
        self.edges.iter().filter(|e| e.to == id).collect()
    }

    /// The edges that leave `id`.
    pub fn outputs(&self, id: &str) -> Vec<&Edge> {
        self.edges.iter().filter(|e| e.from == id).collect()
    }

    /// Check everything a person typed and the shape of the graph (ADR-0033).
    pub fn validate(&self) -> Result<()> {
        let bad = |what: String| Err(Error::InvalidInput(what));
        if self.name.trim().is_empty() || self.name.chars().count() > 80 {
            return bad("The name has 1 to 80 characters.".into());
        }
        if self.description.chars().count() > 280 {
            return bad("The description has at most 280 characters.".into());
        }
        if self.group.chars().count() > 40 {
            return bad("The group has at most 40 characters.".into());
        }
        if self.steps.is_empty() || self.steps.len() > MAX_STEPS {
            return bad(format!("A workflow has 1 to {MAX_STEPS} steps."));
        }
        if self.budget_micros == Some(0) {
            return bad("A budget of zero lets nothing run; leave it empty for no limit.".into());
        }
        let mut ids = BTreeSet::new();
        for step in &self.steps {
            if !valid_step_id(&step.id) {
                return bad(format!(
                    "“{}” is not a valid step id: use 1 to 32 lowercase letters, digits, '-' or '_'.",
                    step.id
                ));
            }
            if !ids.insert(step.id.as_str()) {
                return bad(format!("Two steps are called “{}”.", step.id));
            }
            if step.title.trim().is_empty() || step.title.chars().count() > 80 {
                return bad(format!("The title of step {} has 1 to 80 characters.", step.id));
            }
            validate_kind(step)?;
        }
        if self.triggers().next().is_none() {
            return bad("A workflow needs a trigger: a schedule, a webhook, an event or a manual start.".into());
        }
        let mut seen = BTreeSet::new();
        for edge in &self.edges {
            if !ids.contains(edge.from.as_str()) || !ids.contains(edge.to.as_str()) {
                return bad(format!("The arrow {} → {} names a step that does not exist.", edge.from, edge.to));
            }
            if edge.from == edge.to {
                return bad(format!("Step {} leads into itself.", edge.from));
            }
            if !seen.insert((edge.from.as_str(), edge.to.as_str())) {
                return bad(format!("There are two arrows from {} to {}.", edge.from, edge.to));
            }
            if self.step(&edge.to).is_some_and(|s| s.kind.is_trigger()) {
                return bad(format!("{} is a trigger, so nothing leads into it.", edge.to));
            }
            let from_condition = matches!(self.step(&edge.from).map(|s| &s.kind), Some(StepKind::Condition { .. }));
            match (&edge.label, from_condition) {
                (Some(l), true) if l == "yes" || l == "no" => {}
                (None, true) => return bad(format!("The arrows of condition {} are labelled yes or no.", edge.from)),
                (Some(l), true) => return bad(format!("“{l}” is not a branch: use yes or no.")),
                (Some(_), false) => return bad(format!("Only a condition's arrows have labels (step {}).", edge.from)),
                (None, false) => {}
            }
        }
        for step in &self.steps {
            if matches!(step.kind, StepKind::Condition { .. }) {
                let labels: Vec<&str> = self.outputs(&step.id).iter().filter_map(|e| e.label.as_deref()).collect();
                let yes = labels.iter().filter(|l| **l == "yes").count();
                let no = labels.iter().filter(|l| **l == "no").count();
                if yes > 1 || no > 1 || labels.is_empty() {
                    return bad(format!("Condition {} has a yes arrow and a no arrow, one of each at most.", step.id));
                }
            }
            if !step.kind.is_trigger() && self.inputs(&step.id).is_empty() {
                return bad(format!("Nothing leads into step {}, so it would never run.", step.id));
            }
        }
        // Acyclic, and every step reachable from a trigger (Kahn).
        let mut indegree: HashMap<&str, usize> = self.steps.iter().map(|s| (s.id.as_str(), 0)).collect();
        for edge in &self.edges {
            *indegree.entry(edge.to.as_str()).or_default() += 1;
        }
        let mut ready: Vec<&str> = indegree.iter().filter(|(_, n)| **n == 0).map(|(id, _)| *id).collect();
        let mut visited = 0;
        while let Some(id) = ready.pop() {
            visited += 1;
            for edge in self.edges.iter().filter(|e| e.from == id) {
                let n = indegree.entry(edge.to.as_str()).or_default();
                *n -= 1;
                if *n == 0 {
                    ready.push(edge.to.as_str());
                }
            }
        }
        if visited != self.steps.len() {
            return bad("The arrows form a loop. A workflow goes one way; use a loop step to repeat.".into());
        }
        Ok(())
    }
}

fn text_ok(what: &str, text: &str, max: usize) -> Result<()> {
    if text.trim().is_empty() || text.chars().count() > max {
        return Err(Error::InvalidInput(format!("{what} has 1 to {max} characters.")));
    }
    Ok(())
}

fn timeout_ok(what: &str, ms: Option<u64>) -> Result<()> {
    match ms {
        Some(ms) if !(MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(&ms) => {
            Err(Error::InvalidInput(format!("{what} is between 1 minute and 30 days.")))
        }
        _ => Ok(()),
    }
}

fn validate_kind(step: &Step) -> Result<()> {
    let id = &step.id;
    let bad = |what: String| Err(Error::InvalidInput(what));
    match &step.kind {
        StepKind::Manual => {}
        StepKind::Schedule { when, timezone } => {
            match when {
                When::Every { minutes } => {
                    if *minutes < MIN_EVERY_MINUTES || *minutes > 7 * 24 * 60 {
                        return bad(format!(
                            "Step {id}: a schedule repeats every {MIN_EVERY_MINUTES} minutes to 7 days."
                        ));
                    }
                }
                When::Daily { times, days } => {
                    if times.is_empty() || times.len() > 24 || !times.iter().all(|t| valid_time_of_day(t)) {
                        return bad(format!("Step {id}: give the times as HH:MM, at most 24 of them."));
                    }
                    let unique: BTreeSet<&Day> = days.iter().collect();
                    if unique.len() != days.len() {
                        return bad(format!("Step {id}: a day is listed twice."));
                    }
                }
                When::Cron { expr } => text_ok(&format!("Step {id}: the cron expression"), expr, 100)?,
            }
            if let Some(zone) = timezone {
                text_ok(&format!("Step {id}: the time zone"), zone, 64)?;
            }
        }
        StepKind::Webhook { source, label, filter, .. } => {
            text_ok(&format!("Step {id}: the source"), source, 40)?;
            if let Some(label) = label {
                text_ok(&format!("Step {id}: the label"), label, 60)?;
            }
            if let Some(filter) = filter {
                text_ok(&format!("Step {id}: the filter"), filter, 500)?;
            }
        }
        StepKind::Event { .. } => {}
        StepKind::AfterWorkflow { .. } => {}
        StepKind::Agent { prompt, version, max_cost_micros, .. } => {
            text_ok(&format!("Step {id}: the prompt"), prompt, MAX_TEXT)?;
            if *version == Some(0) {
                return bad(format!("Step {id}: versions start at 1."));
            }
            if *max_cost_micros == Some(0) {
                return bad(format!("Step {id}: a cost limit of zero lets nothing run."));
            }
        }
        StepKind::Team { prompt, max_cost_micros, .. } => {
            text_ok(&format!("Step {id}: the prompt"), prompt, MAX_TEXT)?;
            if *max_cost_micros == Some(0) {
                return bad(format!("Step {id}: a cost limit of zero lets nothing run."));
            }
        }
        StepKind::Condition { expr } => text_ok(&format!("Step {id}: the condition"), expr, 500)?,
        StepKind::Loop { over, prompt, .. } => {
            text_ok(&format!("Step {id}: the list"), over, 500)?;
            text_ok(&format!("Step {id}: the prompt"), prompt, MAX_TEXT)?;
        }
        StepKind::Wait { for_ms, until } => match (for_ms, until) {
            (Some(_), None) => timeout_ok(&format!("Step {id}: the wait"), *for_ms)?,
            (None, Some(time)) if valid_time_of_day(time) => {}
            (None, Some(_)) => return bad(format!("Step {id}: give the time as HH:MM.")),
            _ => return bad(format!("Step {id}: a wait has a duration or a time of day, not both and not neither.")),
        },
        StepKind::Approval { message, show, timeout_ms, remind, .. } => {
            text_ok(&format!("Step {id}: the message"), message, 1000)?;
            if show.len() > 12 {
                return bad(format!("Step {id}: show at most 12 values."));
            }
            timeout_ok(&format!("Step {id}: the time-out"), *timeout_ms)?;
            if let Some(remind) = remind {
                timeout_ok(&format!("Step {id}: the reminder"), Some(remind.after_ms))?;
                text_ok(&format!("Step {id}: the reminder connection"), &remind.connection, 64)?;
                text_ok(&format!("Step {id}: the reminder channel"), &remind.channel, 200)?;
                if let Some(limit) = timeout_ms
                    && remind.after_ms >= *limit
                {
                    return bad(format!("Step {id}: the reminder comes before the time-out."));
                }
            }
        }
        StepKind::Question { question, timeout_ms, .. } => {
            text_ok(&format!("Step {id}: the question"), question, 1000)?;
            timeout_ok(&format!("Step {id}: the time-out"), *timeout_ms)?;
        }
        StepKind::Send { channel, text, .. } => {
            text_ok(&format!("Step {id}: the channel"), channel, 200)?;
            text_ok(&format!("Step {id}: the message"), text, 4000)?;
        }
        StepKind::Http { method, url, body, .. } => {
            if !["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&method.as_str()) {
                return bad(format!("Step {id}: the method is GET, POST, PUT, PATCH or DELETE."));
            }
            text_ok(&format!("Step {id}: the address"), url, 2048)?;
            if let Some(body) = body
                && body.chars().count() > MAX_TEXT
            {
                return bad(format!("Step {id}: the body has at most {MAX_TEXT} characters."));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------------
// Records

/// A person's confirmation that an action runs unattended, recorded with the version it was given for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Confirmation {
    /// The step.
    pub step: String,
    /// The hash of the step's settings when it was confirmed.
    pub hash: String,
    /// Who confirmed.
    pub by: String,
    /// When.
    pub at: Timestamp,
}

/// A workflow: its draft, and whether it is on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Workflow {
    /// Its id.
    pub id: WorkflowId,
    /// The owner.
    pub tenant: TenantId,
    /// Its short name (never changes).
    pub slug: WorkflowSlug,
    /// What a person is editing. Published versions are separate and never change.
    pub draft: WorkflowSpec,
    /// The newest published version, which the next trigger uses; none until the first publish.
    pub live_version: Option<u32>,
    /// Whether triggers start it. Only a published workflow can be on.
    pub enabled: bool,
    /// When it was made.
    pub created_at: Timestamp,
    /// When the draft or the switch last changed.
    pub updated_at: Timestamp,
}

/// One published version of a workflow. Never changed after it is written.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkflowVersion {
    /// The workflow.
    pub workflow: WorkflowId,
    /// 1, 2, 3 …
    pub version: u32,
    /// The hash of the spec.
    pub hash: String,
    /// The definition.
    pub spec: WorkflowSpec,
    /// Why it was published.
    pub note: Option<String>,
    /// The unattended actions a person confirmed for this version.
    #[serde(default)]
    pub confirmations: Vec<Confirmation>,
    /// When it was published.
    pub created_at: Timestamp,
}

/// Who confirms unattended actions when publishing, and when.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confirm {
    /// Who.
    pub by: String,
}

/// Where a workflow run stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    /// Steps are running or ready to.
    Running,
    /// Nothing is running; a person or a timer is awaited.
    Waiting,
    /// Every step is done or skipped.
    Completed,
    /// A step failed or was refused.
    Failed,
    /// A person cancelled it.
    Cancelled,
}

impl RunState {
    /// Whether the run has ended.
    pub fn is_final(self) -> bool {
        matches!(self, RunState::Completed | RunState::Failed | RunState::Cancelled)
    }

    /// The name stored in the database and shown in files.
    pub fn as_str(self) -> &'static str {
        match self {
            RunState::Running => "running",
            RunState::Waiting => "waiting",
            RunState::Completed => "completed",
            RunState::Failed => "failed",
            RunState::Cancelled => "cancelled",
        }
    }

    /// The inverse of [`RunState::as_str`].
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "running" => RunState::Running,
            "waiting" => RunState::Waiting,
            "completed" => RunState::Completed,
            "failed" => RunState::Failed,
            "cancelled" => RunState::Cancelled,
            _ => return None,
        })
    }
}

/// Where one step of a run stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    /// Not reached yet.
    #[default]
    Pending,
    /// Running now.
    Running,
    /// Waiting for a person or a timer.
    Waiting,
    /// Finished.
    Done,
    /// Failed or refused.
    Failed,
    /// On a branch that was not taken.
    Skipped,
}

impl StepState {
    /// Whether nothing more will happen to the step.
    pub fn is_settled(self) -> bool {
        matches!(self, StepState::Done | StepState::Failed | StepState::Skipped)
    }
}

/// What happened to one step of one run.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StepRun {
    /// Where it stands.
    pub state: StepState,
    /// When it started.
    #[serde(default)]
    pub started_at: Option<Timestamp>,
    /// When it ended.
    #[serde(default)]
    pub finished_at: Option<Timestamp>,
    /// What it produced, for the steps after it.
    #[serde(default)]
    pub output: Option<Value>,
    /// Why it failed or was refused.
    #[serde(default)]
    pub error: Option<String>,
    /// A short result for the canvas: the branch a condition took, "5 steps · $0.21".
    #[serde(default)]
    pub note: Option<String>,
    /// The execution an agent or team step created.
    #[serde(default)]
    pub execution: Option<ExecutionId>,
    /// The executions a loop step created, one per item.
    #[serde(default)]
    pub executions: Vec<ExecutionId>,
    /// The wait a person or a timer is being waited for.
    #[serde(default)]
    pub wait: Option<WaitId>,
    /// What the step cost, in millionths of a dollar.
    #[serde(default)]
    pub cost_micros: u64,
}

/// What started a run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fired {
    /// The trigger step.
    pub step: String,
    /// Its chip text.
    pub label: String,
    /// The data it carried: a webhook's body, the ending of a run, a manual input.
    pub payload: Value,
    /// When.
    pub at: Timestamp,
}

/// One run of a workflow.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkflowRun {
    /// Its id.
    pub id: WorkflowRunId,
    /// The owner.
    pub tenant: TenantId,
    /// The workflow.
    pub workflow: WorkflowId,
    /// The workflow's name, kept for lists.
    pub slug: WorkflowSlug,
    /// The published version that ran; none for a test of the draft.
    pub version: Option<u32>,
    /// Whether this is a test run: every action asks first.
    pub test: bool,
    /// The definition that runs, frozen when the run started.
    pub spec: WorkflowSpec,
    /// What started it.
    pub fired: Fired,
    /// Where it stands.
    pub state: RunState,
    /// Every step's record.
    pub steps: BTreeMap<String, StepRun>,
    /// What the run cost so far, in millionths of a dollar.
    pub cost_micros: u64,
    /// Why it failed.
    #[serde(default)]
    pub error: Option<String>,
    /// How many workflows started each other to get here.
    #[serde(default)]
    pub chain: u32,
    /// The run that started this one, when a workflow did.
    #[serde(default)]
    pub from_run: Option<WorkflowRunId>,
    /// When it started.
    pub started_at: Timestamp,
    /// When it ended.
    #[serde(default)]
    pub finished_at: Option<Timestamp>,
    /// Counts every change, so two writers cannot overwrite each other.
    #[serde(default)]
    pub revision: u64,
}

/// What a wait is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitKind {
    /// A person approves or refuses.
    Approval,
    /// A person answers a question.
    Question,
    /// A person approves one call of an action (ADR-0035).
    Action,
    /// A timer.
    Timer,
}

/// How a wait ended, or that it has not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitStatus {
    /// Still waiting.
    Pending,
    /// Approved by a person.
    Approved,
    /// Refused by a person.
    Refused,
    /// Answered by a person.
    Answered,
    /// A timer ran out.
    Elapsed,
    /// The time-out passed.
    TimedOut,
    /// The run was cancelled.
    Cancelled,
}

impl WaitStatus {
    /// The name stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            WaitStatus::Pending => "pending",
            WaitStatus::Approved => "approved",
            WaitStatus::Refused => "refused",
            WaitStatus::Answered => "answered",
            WaitStatus::Elapsed => "elapsed",
            WaitStatus::TimedOut => "timed_out",
            WaitStatus::Cancelled => "cancelled",
        }
    }
}

/// A run stopped for a person or a timer. Not tied to a tool call, so it works without a model (ADR-0035).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkflowWait {
    /// Its id.
    pub id: WaitId,
    /// The owner.
    pub tenant: TenantId,
    /// The run.
    pub run: WorkflowRunId,
    /// The workflow, for lists.
    pub workflow: WorkflowId,
    /// The step.
    pub step: String,
    /// What it waits for.
    pub kind: WaitKind,
    /// The step's title.
    pub title: String,
    /// The question or the request, as the person reads it.
    pub message: String,
    /// What the person is shown, as it was when the wait began: the chosen values, or the call of an action.
    pub shown: Value,
    /// The SHA-256 of `message` and `shown` and the step's hash. A decision names it.
    pub fingerprint: String,
    /// How it stands.
    pub status: WaitStatus,
    /// When a time-out or timer ends it.
    #[serde(default)]
    pub due_at: Option<Timestamp>,
    /// What happens at the time-out.
    #[serde(default)]
    pub then: Then,
    /// The answer taken at a time-out of a question (or `None`).
    #[serde(default)]
    pub default_answer: Option<String>,
    /// When a reminder is due, and whether it was sent.
    #[serde(default)]
    pub remind_at: Option<Timestamp>,
    /// Whether the reminder went out.
    #[serde(default)]
    pub reminded: bool,
    /// When it began.
    pub created_at: Timestamp,
    /// When it ended.
    #[serde(default)]
    pub decided_at: Option<Timestamp>,
    /// Who decided.
    #[serde(default)]
    pub decided_by: Option<String>,
    /// The answer to a question.
    #[serde(default)]
    pub answer: Option<String>,
    /// A note the person added.
    #[serde(default)]
    pub note: Option<String>,
}

/// How a wait ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaitDecision {
    /// The outcome.
    pub status: WaitStatus,
    /// Who decided, when a person did.
    pub by: Option<String>,
    /// The answer to a question.
    pub answer: Option<String>,
    /// A note the person added.
    pub note: Option<String>,
}

impl WaitDecision {
    /// An ending with no person, no answer and no note (a time-out, a timer, a cancel).
    pub fn of(status: WaitStatus) -> Self {
        Self { status, by: None, answer: None, note: None }
    }
}

/// The fingerprint of what a person is shown in a wait.
pub fn wait_fingerprint(message: &str, shown: &Value, step_hash: &str) -> String {
    canonical_hash(&serde_json::json!({ "message": message, "shown": shown, "step": step_hash }))
}

/// When a trigger fires next, and what happened to the fires so far.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerState {
    /// The workflow.
    pub workflow: WorkflowId,
    /// The trigger step.
    pub step: String,
    /// The next fire of a schedule.
    #[serde(default)]
    pub next_at: Option<Timestamp>,
    /// The last time it fired.
    #[serde(default)]
    pub last_fired: Option<Timestamp>,
    /// How many fires were missed while nothing was running.
    #[serde(default)]
    pub missed: u32,
    /// When the newest missed fire was due.
    #[serde(default)]
    pub last_missed: Option<Timestamp>,
}

// ---------------------------------------------------------------------------------------------------------------------
// The store

/// Where workflows, their versions, runs and waits are kept. Every call takes the tenant; nothing crosses tenants.
#[async_trait]
pub trait WorkflowStore: Send + Sync {
    /// The workflows of a tenant, by slug.
    async fn workflows(&self, tenant: &TenantId) -> Result<Vec<Workflow>>;
    /// One workflow.
    async fn workflow(&self, tenant: &TenantId, id: WorkflowId) -> Result<Option<Workflow>>;
    /// One workflow by slug.
    async fn workflow_by_slug(&self, tenant: &TenantId, slug: &WorkflowSlug) -> Result<Option<Workflow>>;
    /// Make a workflow with a draft. It is off, and nothing is published yet. Fails when the slug is taken or the tenant has
    /// [`MAX_WORKFLOWS`].
    async fn create_workflow(
        &self,
        tenant: &TenantId,
        slug: WorkflowSlug,
        draft: WorkflowSpec,
        now: Timestamp,
    ) -> Result<Workflow>;
    /// Replace the draft.
    async fn save_draft(
        &self,
        tenant: &TenantId,
        id: WorkflowId,
        draft: WorkflowSpec,
        now: Timestamp,
    ) -> Result<Workflow>;
    /// Publish the draft as the next version, unless it equals the newest (then nothing is written and the second value is
    /// false). Every unattended action must carry a confirmation: a still-valid one from the newest version, or one given now by
    /// `confirm`.
    async fn publish(
        &self,
        tenant: &TenantId,
        id: WorkflowId,
        note: Option<String>,
        confirm: Option<Confirm>,
        now: Timestamp,
    ) -> Result<(WorkflowVersion, bool)>;
    /// The versions of a workflow, newest first.
    async fn versions(&self, tenant: &TenantId, id: WorkflowId) -> Result<Vec<WorkflowVersion>>;
    /// One version.
    async fn version(&self, tenant: &TenantId, id: WorkflowId, version: u32) -> Result<Option<WorkflowVersion>>;
    /// Put an old version's definition in the draft and publish it as a new version.
    async fn rollback(
        &self,
        tenant: &TenantId,
        id: WorkflowId,
        to: u32,
        confirm: Option<Confirm>,
        now: Timestamp,
    ) -> Result<WorkflowVersion>;
    /// Turn triggers on or off. Only a published workflow can be turned on.
    async fn set_enabled(&self, tenant: &TenantId, id: WorkflowId, enabled: bool, now: Timestamp) -> Result<()>;
    /// Delete a workflow with its versions and trigger state. Its runs and waits stay.
    async fn remove_workflow(&self, tenant: &TenantId, id: WorkflowId) -> Result<bool>;

    /// Write a run. A new run has `revision` 0; an existing one is written only when `expected` is its current revision, which
    /// then goes up by one. Returns false when somebody else changed it first.
    async fn put_run(&self, tenant: &TenantId, run: &WorkflowRun, expected: Option<u64>) -> Result<bool>;
    /// One run.
    async fn run(&self, tenant: &TenantId, id: WorkflowRunId) -> Result<Option<WorkflowRun>>;
    /// Runs, newest first, of one workflow or of all.
    async fn runs(&self, tenant: &TenantId, workflow: Option<WorkflowId>, limit: usize) -> Result<Vec<WorkflowRun>>;
    /// The runs that have not ended.
    async fn active_runs(&self, tenant: &TenantId) -> Result<Vec<WorkflowRun>>;

    /// Record a new wait.
    async fn put_wait(&self, tenant: &TenantId, wait: &WorkflowWait) -> Result<()>;
    /// One wait.
    async fn wait(&self, tenant: &TenantId, id: WaitId) -> Result<Option<WorkflowWait>>;
    /// The waits of a run.
    async fn waits_of(&self, tenant: &TenantId, run: WorkflowRunId) -> Result<Vec<WorkflowWait>>;
    /// The waits still pending, oldest first.
    async fn pending_waits(&self, tenant: &TenantId) -> Result<Vec<WorkflowWait>>;
    /// End a wait that is still pending. Returns the updated wait, or none when it was not pending (answered first).
    async fn settle_wait(
        &self,
        tenant: &TenantId,
        id: WaitId,
        decision: WaitDecision,
        now: Timestamp,
    ) -> Result<Option<WorkflowWait>>;
    /// Mark that a pending wait's reminder was sent.
    async fn mark_reminded(&self, tenant: &TenantId, id: WaitId) -> Result<()>;

    /// The state of every trigger of a tenant.
    async fn trigger_states(&self, tenant: &TenantId) -> Result<Vec<TriggerState>>;
    /// Record the state of one trigger.
    async fn put_trigger_state(&self, tenant: &TenantId, state: &TriggerState) -> Result<()>;
    /// Forget the trigger state of a workflow (when its triggers changed).
    async fn clear_trigger_states(&self, tenant: &TenantId, workflow: WorkflowId) -> Result<()>;
    /// A small value the host keeps, such as how far it has read the events.
    async fn meta(&self, tenant: &TenantId, key: &str) -> Result<Option<String>>;
    /// Keep a small value.
    async fn set_meta(&self, tenant: &TenantId, key: &str, value: &str) -> Result<()>;
    /// Remember that a webhook delivery (named by the sender's own delivery id) reached a workflow. Returns `true` the first time and
    /// `false` for a repeat. A delivery is remembered for `keep_ms`, and at most the newest 10,000 of a workflow are kept.
    async fn record_delivery(
        &self,
        tenant: &TenantId,
        workflow: WorkflowId,
        key: &str,
        now: Timestamp,
        keep_ms: i64,
    ) -> Result<bool>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str) -> Step {
        Step {
            id: id.into(),
            title: format!("Step {id}"),
            x: 0,
            y: 0,
            kind: StepKind::Agent {
                agent: AgentSlug::new("researcher").unwrap(),
                prompt: "Look into it".into(),
                version: None,
                model: None,
                max_cost_micros: None,
            },
        }
    }

    fn condition(id: &str) -> Step {
        Step {
            id: id.into(),
            title: "Is it big?".into(),
            x: 0,
            y: 0,
            kind: StepKind::Condition { expr: "a > 1".into() },
        }
    }

    fn edge(from: &str, to: &str, label: Option<&str>) -> Edge {
        Edge { from: from.into(), to: to.into(), label: label.map(String::from) }
    }

    fn base() -> WorkflowSpec {
        let mut spec = WorkflowSpec::blank("Daily");
        spec.steps.push(agent("a"));
        spec.edges.push(edge("start", "a", None));
        spec
    }

    #[test]
    fn a_blank_workflow_and_a_simple_chain_are_valid() {
        WorkflowSpec::blank("Daily").validate().unwrap();
        base().validate().unwrap();
    }

    #[test]
    fn a_workflow_needs_a_trigger_and_every_step_needs_an_input() {
        let mut spec = base();
        spec.steps.retain(|s| !s.kind.is_trigger());
        spec.edges.clear();
        assert!(spec.validate().unwrap_err().to_string().contains("needs a trigger"));

        let mut spec = base();
        spec.steps.push(agent("orphan"));
        assert!(spec.validate().unwrap_err().to_string().contains("never run"));
    }

    #[test]
    fn a_cycle_is_refused() {
        let mut spec = base();
        spec.steps.push(agent("b"));
        spec.edges.push(edge("a", "b", None));
        spec.edges.push(edge("b", "a", None));
        assert!(spec.validate().unwrap_err().to_string().contains("loop"));
    }

    #[test]
    fn nothing_leads_into_a_trigger_and_a_step_does_not_lead_into_itself() {
        let mut spec = base();
        spec.edges.push(edge("a", "start", None));
        assert!(spec.validate().unwrap_err().to_string().contains("trigger"));
        let mut spec = base();
        spec.edges.push(edge("a", "a", None));
        assert!(spec.validate().unwrap_err().to_string().contains("itself"));
    }

    #[test]
    fn a_condition_labels_its_arrows_yes_and_no() {
        let mut spec = base();
        spec.steps.push(condition("c"));
        spec.steps.push(agent("b"));
        spec.edges.push(edge("a", "c", None));
        spec.edges.push(edge("c", "b", None));
        assert!(spec.validate().unwrap_err().to_string().contains("labelled yes or no"));
        spec.edges.pop();
        spec.edges.push(edge("c", "b", Some("maybe")));
        assert!(spec.validate().unwrap_err().to_string().contains("yes or no"));
        spec.edges.pop();
        spec.edges.push(edge("c", "b", Some("yes")));
        spec.validate().unwrap();
        spec.edges.push(edge("a", "b", Some("yes")));
        assert!(spec.validate().unwrap_err().to_string().contains("Only a condition"));
    }

    #[test]
    fn size_limits_hold() {
        let mut spec = base();
        for i in 0..MAX_STEPS {
            spec.steps.push(agent(&format!("s{i}")));
            spec.edges.push(edge("start", &format!("s{i}"), None));
        }
        assert!(spec.validate().unwrap_err().to_string().contains("1 to 50"));
    }

    #[test]
    fn schedules_and_waits_are_checked() {
        let mut spec = WorkflowSpec::blank("x");
        spec.steps[0].kind = StepKind::Schedule { when: When::Every { minutes: 1 }, timezone: None };
        assert!(spec.validate().is_err());
        spec.steps[0].kind =
            StepKind::Schedule { when: When::Daily { times: vec!["25:00".into()], days: vec![] }, timezone: None };
        assert!(spec.validate().is_err());
        spec.steps[0].kind = StepKind::Schedule {
            when: When::Daily { times: vec!["08:00".into()], days: vec![Day::Mon] },
            timezone: None,
        };
        spec.validate().unwrap();

        let mut spec = base();
        spec.steps.push(Step {
            id: "w".into(),
            title: "Wait".into(),
            x: 0,
            y: 0,
            kind: StepKind::Wait { for_ms: Some(1000), until: None },
        });
        spec.edges.push(edge("a", "w", None));
        assert!(spec.validate().unwrap_err().to_string().contains("between 1 minute"));
    }

    #[test]
    fn the_chip_reads_like_the_design_system() {
        let weekdays =
            When::Daily { times: vec!["08:00".into()], days: vec![Day::Mon, Day::Tue, Day::Wed, Day::Thu, Day::Fri] };
        assert_eq!(weekdays.describe(), "Weekdays 08:00");
        assert_eq!(When::Daily { times: vec!["16:00".into()], days: vec![Day::Fri] }.describe(), "Fridays 16:00");
        assert_eq!(When::Daily { times: vec!["09:30".into()], days: vec![] }.describe(), "Daily 09:30");
        assert_eq!(When::Every { minutes: 15 }.describe(), "Every 15 minutes");
        assert_eq!(When::Every { minutes: 120 }.describe(), "Every 2 hours");
        let hook = StepKind::Webhook {
            source: "GitHub".into(),
            label: Some("issue labelled".into()),
            filter: None,
            signed: true,
        };
        assert_eq!(hook.trigger_label().unwrap(), "GitHub · issue labelled");
        assert_eq!(StepKind::Manual.trigger_label().unwrap(), "Manual");
        assert!(agent("a").kind.trigger_label().is_none());
    }

    #[test]
    fn moving_a_step_does_not_change_the_hash_but_editing_one_does() {
        let spec = base();
        let mut moved = spec.clone();
        moved.steps[1].x = 400;
        assert_eq!(spec.hash(), moved.hash());
        let mut edited = spec.clone();
        edited.steps[1].title = "Something else".into();
        assert_ne!(spec.hash(), edited.hash());
        assert_eq!(spec.hash(), spec.hash());
    }

    #[test]
    fn unattended_actions_are_found_and_their_hash_ignores_the_canvas() {
        let mut spec = base();
        let send = Step {
            id: "m".into(),
            title: "Post to #ops".into(),
            x: 0,
            y: 0,
            kind: StepKind::Send {
                service: Service::Slack,
                connection: None,
                channel: "#ops".into(),
                text: "hi".into(),
                ask: Ask::Never,
            },
        };
        spec.steps.push(send.clone());
        spec.edges.push(edge("a", "m", None));
        assert_eq!(spec.unattended().len(), 1);
        let mut moved = send.clone();
        moved.x = 99;
        assert_eq!(send.hash(), moved.hash());
        let mut changed = send;
        if let StepKind::Send { text, .. } = &mut changed.kind {
            *text = "different".into();
        }
        assert_ne!(changed.hash(), moved.hash());
    }

    #[test]
    fn files_refuse_unknown_keys() {
        let text = r#"{"name":"x","steps":[{"id":"s","title":"S","kind":{"type":"manual"}}],"nope":1}"#;
        assert!(serde_json::from_str::<WorkflowSpec>(text).is_err());
        let text = r#"{"name":"x","steps":[{"id":"s","title":"S","kind":{"type":"event","on":"any","extra":1}}]}"#;
        assert!(serde_json::from_str::<WorkflowSpec>(text).is_err());
        let text = r#"{"name":"x","steps":[{"id":"s","title":"S","kind":{"type":"manual"}}]}"#;
        serde_json::from_str::<WorkflowSpec>(text).unwrap();
    }

    #[test]
    fn slugs_follow_the_agent_rules() {
        assert!(WorkflowSlug::new("incident-triage").is_ok());
        assert!(WorkflowSlug::new("Incident").is_err());
        assert!(WorkflowSlug::new("a--b").is_err());
        assert!(WorkflowSlug::new("").is_err());
    }

    #[test]
    fn a_fingerprint_changes_with_what_is_shown() {
        let a = wait_fingerprint("ok?", &serde_json::json!({"x": 1}), "h");
        let b = wait_fingerprint("ok?", &serde_json::json!({"x": 2}), "h");
        assert_ne!(a, b);
        assert_eq!(a, wait_fingerprint("ok?", &serde_json::json!({"x": 1}), "h"));
    }
}
