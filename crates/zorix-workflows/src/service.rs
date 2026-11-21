//! What a screen or a command asks of workflows: the list with its health, the figures, files, an outline to compare versions
//! with, the starters, and a check that names what a workflow needs and the computer does not have yet.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use zorix_core::{
    AgentSlug, AgentStore, Clock, Confirm, ConnectionKind, Error, IntegrationStore, Result, RunState, Service,
    StepKind, TenantId, Timestamp, WaitKind, Workflow, WorkflowId, WorkflowRun, WorkflowSlug, WorkflowSpec,
    WorkflowStore, WorkflowVersion,
};
use zorix_hub::Hub;

/// The format name written in every file.
pub const FORMAT: &str = "workflow/1";
/// Runs shown as bars in the list.
pub const BARS: usize = 14;

/// A workflow as a file: `<slug>.workflow.json`. Strict about unknown keys, and written the same way every time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowFile {
    /// The format and its version.
    pub zorix: String,
    /// The workflow's short name.
    pub slug: WorkflowSlug,
    /// The version the file was exported from, for information.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    /// The definition.
    pub workflow: WorkflowSpec,
}

impl WorkflowFile {
    /// Read a file.
    pub fn parse(text: &str) -> Result<Self> {
        let file: WorkflowFile =
            serde_json::from_str(text).map_err(|e| Error::InvalidInput(format!("This is not a workflow file: {e}")))?;
        if file.zorix != FORMAT {
            return Err(Error::InvalidInput(format!(
                "This file is “{}”; this version of ZORIX reads “{FORMAT}”.",
                file.zorix
            )));
        }
        Ok(file)
    }

    /// Write a file.
    pub fn render(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).unwrap_or_default();
        text.push('\n');
        text
    }
}

/// A trigger as the list shows it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TriggerInfo {
    /// The step.
    pub step: String,
    /// `schedule`, `webhook`, `event`, `after_workflow` or `manual`.
    pub kind: &'static str,
    /// The chip's text.
    pub label: String,
}

/// One bar of the last runs.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Bar {
    /// The run.
    pub run: WorkflowRunIdText,
    /// Where it ended, or is.
    pub state: RunState,
    /// Whether it waited for a person at some point.
    pub waited: bool,
}

/// A run id as text, so the interface does not need the id type.
pub type WorkflowRunIdText = String;

/// How a workflow is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Health {
    /// Switched off.
    Paused,
    /// Runs are waiting for a person.
    Waiting {
        /// How many.
        count: u32,
    },
    /// Some of the last runs failed.
    Failed {
        /// How many of them.
        count: u32,
    },
    /// Nothing wrong.
    Healthy,
}

/// The health of a workflow from whether it is on and how its last runs ended.
pub fn health(enabled: bool, waiting: u32, recent: &[RunState]) -> Health {
    if !enabled {
        return Health::Paused;
    }
    if waiting > 0 {
        return Health::Waiting { count: waiting };
    }
    let failed = recent.iter().filter(|s| **s == RunState::Failed).count() as u32;
    if failed > 0 { Health::Failed { count: failed } } else { Health::Healthy }
}

/// One row of the workflows list.
#[derive(Clone, Debug, Serialize)]
pub struct Card {
    /// The workflow.
    pub workflow: Workflow,
    /// What starts it, from the live version (or the draft before the first publish).
    pub triggers: Vec<TriggerInfo>,
    /// The last runs, oldest first.
    pub bars: Vec<Bar>,
    /// How it is doing.
    pub health: Health,
    /// Whether the draft differs from the live version.
    pub unpublished_changes: bool,
    /// Fires missed while nothing was running, over all its schedules.
    pub missed: u32,
}

/// The four figures above the list.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Figures {
    /// Workflows that are on.
    pub active: u32,
    /// Workflows that are off.
    pub paused: u32,
    /// Runs started in the last seven days.
    pub runs_week: u32,
    /// Of the runs that ended in the last seven days, the share that finished well (0 to 100), when any ended.
    pub finished_percent: Option<f64>,
    /// Runs stopped for a person.
    pub waiting: u32,
    /// How long the oldest of them has waited, in milliseconds.
    pub oldest_wait_ms: Option<u64>,
    /// Minutes of work saved by the runs that finished in the last seven days, when at least one workflow says how much a run
    /// saves. None means "nobody said", never zero.
    pub saved_minutes: Option<u64>,
}

/// What a workflow needs that this computer does not have.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Missing {
    /// The step.
    pub step: String,
    /// `agent`, `team` or `connection`.
    pub kind: &'static str,
    /// Its name, or what kind of connection.
    pub name: String,
}

/// A starter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Template {
    /// Its id, and the name of the workflow it makes.
    pub id: String,
    /// What people read.
    pub name: String,
    /// What it does.
    pub description: String,
    /// Its group.
    pub group: String,
    /// The trigger chips.
    pub triggers: Vec<String>,
    /// How many steps.
    pub steps: usize,
}

const TEMPLATES: [(&str, &str); 3] = [
    ("daily-digest", include_str!("../starters/daily-digest.workflow.json")),
    ("incident-triage", include_str!("../starters/incident-triage.workflow.json")),
    ("weekly-report", include_str!("../starters/weekly-report.workflow.json")),
];

/// One line of a comparison.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DiffLine {
    /// `same`, `added` or `removed`.
    pub kind: &'static str,
    /// The line.
    pub text: String,
}

/// What an import did.
#[derive(Clone, Debug, Serialize)]
pub struct Imported {
    /// The workflow.
    pub workflow: Workflow,
    /// Whether it did not exist before.
    pub created: bool,
    /// Whether the draft changed.
    pub changed: bool,
}

/// Everything the interfaces ask of workflows that does not need a running engine, over the store and the hub.
#[derive(Clone)]
pub struct WorkflowService {
    store: Arc<dyn WorkflowStore>,
    hub: Arc<Hub>,
    tenant: TenantId,
    clock: Arc<dyn Clock>,
}

impl WorkflowService {
    /// A service for one tenant.
    pub fn new(store: Arc<dyn WorkflowStore>, tenant: TenantId, hub: Arc<Hub>, clock: Arc<dyn Clock>) -> Self {
        Self { store, tenant, hub, clock }
    }

    /// The store.
    pub fn store(&self) -> &Arc<dyn WorkflowStore> {
        &self.store
    }

    /// The tenant.
    pub fn tenant(&self) -> &TenantId {
        &self.tenant
    }

    /// A workflow by id or name.
    pub async fn find(&self, key: &str) -> Result<Workflow> {
        if let Ok(id) = key.parse::<WorkflowId>()
            && let Some(found) = self.store.workflow(&self.tenant, id).await?
        {
            return Ok(found);
        }
        let slug = WorkflowSlug::new(key).map_err(Error::InvalidInput)?;
        self.store.workflow_by_slug(&self.tenant, &slug).await?.ok_or_else(|| {
            Error::Missing(format!("There is no workflow called {key}. `zorix workflow list` shows them."))
        })
    }

    // -----------------------------------------------------------------------------------------------------------------
    // The list

    /// Every workflow as a row of the list.
    pub async fn cards(&self) -> Result<Vec<Card>> {
        let workflows = self.store.workflows(&self.tenant).await?;
        let triggers = self.store.trigger_states(&self.tenant).await?;
        let all_runs = self.store.runs(&self.tenant, None, 1000).await?;
        let waiting = self.waiting_by_workflow(&all_runs);
        let mut cards = Vec::new();
        for workflow in workflows {
            let live = match workflow.live_version {
                Some(v) => self.store.version(&self.tenant, workflow.id, v).await?,
                None => None,
            };
            let spec = live.as_ref().map(|v| &v.spec).unwrap_or(&workflow.draft);
            let mut recent: Vec<&WorkflowRun> =
                all_runs.iter().filter(|r| r.workflow == workflow.id && !r.test).take(BARS).collect();
            recent.reverse();
            let states: Vec<RunState> = recent.iter().map(|r| r.state).collect();
            let missed = triggers.iter().filter(|t| t.workflow == workflow.id).map(|t| t.missed).sum();
            cards.push(Card {
                triggers: trigger_infos(spec),
                bars: recent
                    .iter()
                    .map(|r| Bar {
                        run: r.id.to_string(),
                        state: r.state,
                        waited: r.steps.values().any(|s| s.wait.is_some()),
                    })
                    .collect(),
                health: health(workflow.enabled, waiting.get(&workflow.id).copied().unwrap_or(0), &states),
                unpublished_changes: live.as_ref().is_none_or(|v| v.hash != workflow.draft.hash()),
                missed,
                workflow,
            });
        }
        Ok(cards)
    }

    fn waiting_by_workflow(&self, runs: &[WorkflowRun]) -> BTreeMap<WorkflowId, u32> {
        let mut out = BTreeMap::new();
        for run in runs.iter().filter(|r| r.state == RunState::Waiting) {
            *out.entry(run.workflow).or_default() += 1;
        }
        out
    }

    /// The figures above the list.
    pub async fn figures(&self) -> Result<Figures> {
        let now = self.clock.now();
        let week_ago = Timestamp(now.millis() - 7 * 24 * 3600 * 1000);
        let workflows = self.store.workflows(&self.tenant).await?;
        let runs = self.store.runs(&self.tenant, None, 1000).await?;
        let this_week: Vec<&WorkflowRun> = runs.iter().filter(|r| r.started_at >= week_ago && !r.test).collect();
        let ended: Vec<&&WorkflowRun> =
            this_week.iter().filter(|r| matches!(r.state, RunState::Completed | RunState::Failed)).collect();
        let finished = ended.iter().filter(|r| r.state == RunState::Completed).count();
        let waiting: Vec<&WorkflowRun> = runs.iter().filter(|r| r.state == RunState::Waiting).collect();
        let saves = |r: &&&WorkflowRun| r.spec.saves_minutes.map(u64::from);
        let saved_minutes = {
            let sums: Vec<u64> = ended.iter().filter(|r| r.state == RunState::Completed).filter_map(saves).collect();
            (!sums.is_empty()).then(|| sums.iter().sum())
        };
        let mut oldest = None;
        for run in &waiting {
            for wait in self.store.waits_of(&self.tenant, run.id).await? {
                if wait.status == zorix_core::WaitStatus::Pending {
                    let age = now.since(wait.created_at);
                    oldest = Some(oldest.map_or(age, |o: u64| o.max(age)));
                }
            }
        }
        Ok(Figures {
            active: workflows.iter().filter(|w| w.enabled).count() as u32,
            paused: workflows.iter().filter(|w| !w.enabled).count() as u32,
            runs_week: this_week.len() as u32,
            finished_percent: (!ended.is_empty())
                .then(|| (finished as f64 / ended.len() as f64 * 1000.0).round() / 10.0),
            waiting: waiting.len() as u32,
            oldest_wait_ms: oldest,
            saved_minutes,
        })
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Editing

    /// Make a workflow. The draft is checked completely first.
    pub async fn create(&self, slug: WorkflowSlug, draft: WorkflowSpec) -> Result<Workflow> {
        crate::check::validate(&draft).map_err(Error::InvalidInput)?;
        self.store.create_workflow(&self.tenant, slug, draft, self.clock.now()).await
    }

    /// Replace the draft. Triggers that changed lose their schedule state, so the next fire is computed from the new settings.
    pub async fn save_draft(&self, id: WorkflowId, draft: WorkflowSpec) -> Result<Workflow> {
        crate::check::validate(&draft).map_err(Error::InvalidInput)?;
        self.store.save_draft(&self.tenant, id, draft, self.clock.now()).await
    }

    /// Publish the draft as the next version.
    pub async fn publish(
        &self,
        id: WorkflowId,
        note: Option<String>,
        confirm: Option<Confirm>,
    ) -> Result<(WorkflowVersion, bool)> {
        let workflow =
            self.store.workflow(&self.tenant, id).await?.ok_or_else(|| Error::Missing(format!("workflow {id}")))?;
        crate::check::validate(&workflow.draft).map_err(Error::InvalidInput)?;
        let published = self.store.publish(&self.tenant, id, note, confirm, self.clock.now()).await?;
        if published.1 {
            // Schedules are computed again from the new settings.
            self.store.clear_trigger_states(&self.tenant, id).await?;
        }
        Ok(published)
    }

    /// Publish an old version again as a new one.
    pub async fn rollback(&self, id: WorkflowId, to: u32, confirm: Option<Confirm>) -> Result<WorkflowVersion> {
        let version = self.store.rollback(&self.tenant, id, to, confirm, self.clock.now()).await?;
        self.store.clear_trigger_states(&self.tenant, id).await?;
        Ok(version)
    }

    /// Turn triggers on or off.
    pub async fn set_enabled(&self, id: WorkflowId, enabled: bool) -> Result<()> {
        self.store.set_enabled(&self.tenant, id, enabled, self.clock.now()).await?;
        self.store.clear_trigger_states(&self.tenant, id).await
    }

    /// Delete a workflow.
    pub async fn remove(&self, id: WorkflowId) -> Result<bool> {
        self.store.remove_workflow(&self.tenant, id).await
    }

    /// The unattended steps a person is asked to confirm before the draft is published, minus the ones already confirmed.
    pub async fn to_confirm(&self, id: WorkflowId) -> Result<Vec<String>> {
        let workflow =
            self.store.workflow(&self.tenant, id).await?.ok_or_else(|| Error::Missing(format!("workflow {id}")))?;
        let kept: Vec<(String, String)> = match workflow.live_version {
            Some(v) => self
                .store
                .version(&self.tenant, id, v)
                .await?
                .map(|v| v.confirmations.into_iter().map(|c| (c.step, c.hash)).collect())
                .unwrap_or_default(),
            None => Vec::new(),
        };
        Ok(workflow
            .draft
            .unattended()
            .into_iter()
            .filter(|s| !kept.iter().any(|(step, hash)| *step == s.id && *hash == s.hash()))
            .map(|s| s.id.clone())
            .collect())
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Files, comparison, starters

    /// A workflow as a file: the draft, or a published version.
    pub async fn export(&self, id: WorkflowId, version: Option<u32>) -> Result<String> {
        let workflow =
            self.store.workflow(&self.tenant, id).await?.ok_or_else(|| Error::Missing(format!("workflow {id}")))?;
        let (spec, number) = match version {
            Some(v) => {
                let found = self
                    .store
                    .version(&self.tenant, id, v)
                    .await?
                    .ok_or_else(|| Error::Missing(format!("version {v} of {}", workflow.slug)))?;
                (found.spec, Some(v))
            }
            None => (workflow.draft.clone(), None),
        };
        Ok(WorkflowFile { zorix: FORMAT.into(), slug: workflow.slug, version: number, workflow: spec }.render())
    }

    /// Read a file into the draft of the workflow it names, making the workflow when it is new. Nothing is published.
    pub async fn import(&self, text: &str) -> Result<Imported> {
        let file = WorkflowFile::parse(text)?;
        crate::check::validate(&file.workflow).map_err(Error::InvalidInput)?;
        match self.store.workflow_by_slug(&self.tenant, &file.slug).await? {
            None => {
                let workflow = self.create(file.slug, file.workflow).await?;
                Ok(Imported { workflow, created: true, changed: true })
            }
            Some(existing) => {
                let changed = existing.draft.hash() != file.workflow.hash();
                let workflow = if changed { self.save_draft(existing.id, file.workflow).await? } else { existing };
                Ok(Imported { workflow, created: false, changed })
            }
        }
    }

    /// A readable account of a definition, one fact per line, so a comparison of two versions shows what changed. Where steps
    /// sit on the canvas is left out.
    pub fn outline(spec: &WorkflowSpec) -> String {
        outline(spec)
    }

    /// The lines that differ between two definitions.
    pub fn diff(old: &WorkflowSpec, new: &WorkflowSpec) -> Vec<DiffLine> {
        let (a, b) = (outline(old), outline(new));
        TextDiff::from_lines(&a, &b)
            .iter_all_changes()
            .map(|change| DiffLine {
                kind: match change.tag() {
                    ChangeTag::Equal => "same",
                    ChangeTag::Insert => "added",
                    ChangeTag::Delete => "removed",
                },
                text: change.value().trim_end_matches('\n').to_string(),
            })
            .collect()
    }

    /// The starters.
    pub fn templates() -> Vec<Template> {
        TEMPLATES
            .iter()
            .filter_map(|(id, text)| {
                let file = WorkflowFile::parse(text).ok()?;
                Some(Template {
                    id: (*id).to_string(),
                    name: file.workflow.name.clone(),
                    description: file.workflow.description.clone(),
                    group: file.workflow.group.clone(),
                    triggers: file.workflow.triggers().filter_map(|s| s.kind.trigger_label()).collect(),
                    steps: file.workflow.steps.len(),
                })
            })
            .collect()
    }

    /// Make a workflow from a starter: a draft that is off, for the person to read and publish.
    pub async fn add_template(&self, id: &str) -> Result<Workflow> {
        let (_, text) = TEMPLATES.iter().find(|(name, _)| *name == id).ok_or_else(|| {
            Error::Missing(format!("There is no starter called {id}. `zorix workflow templates` lists them."))
        })?;
        let file = WorkflowFile::parse(text)?;
        self.create(file.slug, file.workflow).await
    }

    // -----------------------------------------------------------------------------------------------------------------
    // What is missing

    /// What the workflow names that does not exist here: agents, teams, and a chat connection for each message.
    pub async fn missing(&self, spec: &WorkflowSpec) -> Result<Vec<Missing>> {
        let connections = self.hub.store.connections(&self.tenant).await?;
        let mut out = Vec::new();
        let gone_agent = |slug: &AgentSlug| {
            let agents = self.hub.agents.clone();
            let tenant = self.tenant.clone();
            let slug = slug.clone();
            async move { Ok::<bool, Error>(agents.agent_by_slug(&tenant, &slug).await?.is_none()) }
        };
        for step in &spec.steps {
            match &step.kind {
                StepKind::Agent { agent, .. } | StepKind::Loop { agent, .. } if gone_agent(agent).await? => {
                    out.push(Missing { step: step.id.clone(), kind: "agent", name: agent.to_string() });
                }
                StepKind::Team { team, .. } => {
                    if self.hub.agents.team(&self.tenant, team).await?.is_none() {
                        out.push(Missing { step: step.id.clone(), kind: "team", name: team.to_string() });
                    }
                }
                StepKind::Send { service, connection, .. } => {
                    let wanted = match service {
                        Service::Slack => ConnectionKind::Slack,
                        Service::Discord => ConnectionKind::Discord,
                    };
                    let found = connections
                        .iter()
                        .any(|c| c.kind == wanted && connection.as_deref().is_none_or(|name| c.name == name));
                    if !found {
                        let name = connection.clone().unwrap_or_else(|| format!("{service:?}").to_lowercase());
                        out.push(Missing { step: step.id.clone(), kind: "connection", name });
                    }
                }
                _ => {}
            }
        }
        Ok(out)
    }

    /// The pending waits of one run, newest first, for the run view.
    pub async fn waits_of(&self, run: zorix_core::WorkflowRunId) -> Result<Vec<zorix_core::WorkflowWait>> {
        let mut waits = self.store.waits_of(&self.tenant, run).await?;
        waits.sort_by_key(|w| std::cmp::Reverse(w.created_at));
        Ok(waits)
    }

    /// What step ran how long, and how long people took, over the last runs of a workflow: the inspector's "last 5 runs of this
    /// step" and "median wait".
    pub async fn step_history(&self, workflow: WorkflowId, step: &str, limit: usize) -> Result<StepHistory> {
        let runs = self.store.runs(&self.tenant, Some(workflow), 100).await?;
        let mut outcomes = Vec::new();
        let mut waits = Vec::new();
        for run in runs.iter().filter(|r| !r.test) {
            let Some(record) = run.steps.get(step) else { continue };
            if !record.state.is_settled() {
                continue;
            }
            if outcomes.len() < limit {
                outcomes.push(record.state);
            }
            if let Some(wait) = record.wait
                && let Some(w) = self.store.wait(&self.tenant, wait).await?
                && let (WaitKind::Approval | WaitKind::Question | WaitKind::Action, Some(done)) = (w.kind, w.decided_at)
            {
                waits.push(done.since(w.created_at));
            }
        }
        outcomes.reverse();
        waits.sort_unstable();
        let median_wait_ms = (!waits.is_empty()).then(|| waits[waits.len() / 2]);
        Ok(StepHistory { outcomes, median_wait_ms })
    }
}

/// The last runs of one step.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StepHistory {
    /// How the step ended, oldest first.
    pub outcomes: Vec<zorix_core::StepState>,
    /// The median time people took to answer it, when people did.
    pub median_wait_ms: Option<u64>,
}

fn trigger_infos(spec: &WorkflowSpec) -> Vec<TriggerInfo> {
    spec.triggers()
        .map(|s| TriggerInfo {
            step: s.id.clone(),
            kind: s.kind.name(),
            label: s.kind.trigger_label().unwrap_or_default(),
        })
        .collect()
}

/// Rendered as the text a comparison reads.
fn outline(spec: &WorkflowSpec) -> String {
    let mut out = String::new();
    let line = |out: &mut String, text: String| {
        out.push_str(&text);
        out.push('\n');
    };
    line(&mut out, format!("name: {}", spec.name));
    if !spec.description.is_empty() {
        line(&mut out, format!("description: {}", spec.description));
    }
    if !spec.group.is_empty() {
        line(&mut out, format!("group: {}", spec.group));
    }
    if let Some(budget) = spec.budget_micros {
        line(&mut out, format!("budget: ${:.2} per run", budget as f64 / 1_000_000.0));
    }
    if let Some(minutes) = spec.saves_minutes {
        line(&mut out, format!("saves: {minutes} min per run"));
    }
    for step in &spec.steps {
        line(&mut out, String::new());
        line(&mut out, format!("{} · {} ({})", step.id, step.title, step.kind.name()));
        for (key, value) in settings(&step.kind) {
            let mut lines = value.lines();
            line(&mut out, format!("  {key}: {}", lines.next().unwrap_or_default()));
            for more in lines {
                line(&mut out, format!("    {more}"));
            }
        }
    }
    if !spec.edges.is_empty() {
        line(&mut out, String::new());
    }
    for edge in &spec.edges {
        match &edge.label {
            Some(label) => line(&mut out, format!("{} → {} ({label})", edge.from, edge.to)),
            None => line(&mut out, format!("{} → {}", edge.from, edge.to)),
        }
    }
    out
}

fn ms(ms: u64) -> String {
    let minutes = ms / 60_000;
    if minutes >= 60 * 24 && minutes.is_multiple_of(60 * 24) {
        format!("{} d", minutes / (60 * 24))
    } else if minutes >= 60 && minutes.is_multiple_of(60) {
        format!("{} h", minutes / 60)
    } else {
        format!("{minutes} min")
    }
}

fn ask(ask: zorix_core::Ask) -> &'static str {
    match ask {
        zorix_core::Ask::Always => "asks first",
        zorix_core::Ask::Never => "runs without asking",
    }
}

fn settings(kind: &StepKind) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = Vec::new();
    match kind {
        StepKind::Manual => {}
        StepKind::Schedule { when, timezone } => {
            out.push(("when", when.describe()));
            if let Some(zone) = timezone {
                out.push(("time zone", zone.clone()));
            }
        }
        StepKind::Webhook { source, label, filter, signed } => {
            out.push(("source", source.clone()));
            if let Some(label) = label {
                out.push(("label", label.clone()));
            }
            if let Some(filter) = filter {
                out.push(("filter", filter.clone()));
            }
            out.push(("signed", signed.to_string()));
        }
        StepKind::Event { on, agent } => {
            out.push(("on", format!("{on:?}").to_lowercase()));
            if let Some(agent) = agent {
                out.push(("agent", agent.to_string()));
            }
        }
        StepKind::AfterWorkflow { workflow, on } => {
            out.push(("workflow", workflow.to_string()));
            out.push(("on", format!("{on:?}").to_lowercase()));
        }
        StepKind::Agent { agent, prompt, version, model, max_cost_micros } => {
            out.push(("agent", agent.to_string()));
            if let Some(version) = version {
                out.push(("version", version.to_string()));
            }
            if let Some(model) = model {
                out.push(("model", model.clone()));
            }
            if let Some(cap) = max_cost_micros {
                out.push(("cost limit", format!("${:.2}", *cap as f64 / 1_000_000.0)));
            }
            out.push(("prompt", prompt.clone()));
        }
        StepKind::Team { team, prompt, model, max_cost_micros } => {
            out.push(("team", team.to_string()));
            if let Some(model) = model {
                out.push(("model", model.clone()));
            }
            if let Some(cap) = max_cost_micros {
                out.push(("cost limit", format!("${:.2}", *cap as f64 / 1_000_000.0)));
            }
            out.push(("prompt", prompt.clone()));
        }
        StepKind::Condition { expr } => out.push(("if", expr.clone())),
        StepKind::Loop { over, agent, prompt } => {
            out.push(("for each", over.clone()));
            out.push(("agent", agent.to_string()));
            out.push(("prompt", prompt.clone()));
        }
        StepKind::Wait { for_ms, until } => {
            if let Some(for_ms) = for_ms {
                out.push(("for", ms(*for_ms)));
            }
            if let Some(until) = until {
                out.push(("until", until.clone()));
            }
        }
        StepKind::Approval { message, show, timeout_ms, then, remind } => {
            out.push(("asks", message.clone()));
            if !show.is_empty() {
                out.push(("shows", show.join(", ")));
            }
            if let Some(timeout) = timeout_ms {
                out.push(("time-out", format!("{}, then {}", ms(*timeout), format!("{then:?}").to_lowercase())));
            }
            if let Some(remind) = remind {
                out.push((
                    "reminds",
                    format!("after {} in {} on {}", ms(remind.after_ms), remind.channel, remind.connection),
                ));
            }
        }
        StepKind::Question { question, default, timeout_ms, then } => {
            out.push(("asks", question.clone()));
            if let Some(default) = default {
                out.push(("default", default.clone()));
            }
            if let Some(timeout) = timeout_ms {
                out.push(("time-out", format!("{}, then {}", ms(*timeout), format!("{then:?}").to_lowercase())));
            }
        }
        StepKind::Send { service, connection, channel, text, ask: a } => {
            out.push(("service", format!("{service:?}").to_lowercase()));
            if let Some(connection) = connection {
                out.push(("connection", connection.clone()));
            }
            out.push(("channel", channel.clone()));
            out.push(("message", text.clone()));
            out.push(("ask", ask(*a).into()));
        }
        StepKind::Http { method, url, body, ask: a } => {
            out.push(("request", format!("{method} {url}")));
            if let Some(body) = body {
                out.push(("body", body.clone()));
            }
            out.push(("ask", ask(*a).into()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use zorix_core::{Edge, Step};

    use super::*;

    #[test]
    fn health_puts_waiting_before_failed_and_a_paused_workflow_is_paused() {
        use RunState::*;
        assert_eq!(health(false, 3, &[Failed]), Health::Paused);
        assert_eq!(health(true, 1, &[Failed, Completed]), Health::Waiting { count: 1 });
        assert_eq!(health(true, 0, &[Failed, Completed, Failed]), Health::Failed { count: 2 });
        assert_eq!(health(true, 0, &[Completed, Cancelled]), Health::Healthy);
        assert_eq!(health(true, 0, &[]), Health::Healthy);
    }

    #[test]
    fn every_starter_is_a_valid_workflow_in_the_current_format() {
        let templates = WorkflowService::templates();
        assert_eq!(templates.len(), 3);
        for (id, text) in TEMPLATES {
            let file = WorkflowFile::parse(text).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(file.slug.as_str(), id);
            crate::check::validate(&file.workflow).unwrap_or_else(|e| panic!("{id}: {e}"));
            // A file is written the way it is read, so an export of an import is the same text.
            assert_eq!(WorkflowFile::parse(&file.render()).unwrap(), file);
        }
        assert!(templates.iter().any(|t| t.triggers == vec!["Weekdays 08:00".to_string()]));
        assert!(templates.iter().any(|t| t.triggers == vec!["GitHub · issue labelled".to_string()]));
        assert!(templates.iter().any(|t| t.triggers == vec!["Fridays 16:00".to_string()]));
    }

    #[test]
    fn a_file_must_name_its_format_and_know_every_key() {
        let ok = TEMPLATES[0].1;
        assert!(
            WorkflowFile::parse(&ok.replace("workflow/1", "workflow/2"))
                .unwrap_err()
                .to_string()
                .contains("workflow/2")
        );
        assert!(WorkflowFile::parse(&ok.replace("\"slug\"", "\"extra\": 1, \"slug\"")).is_err());
        assert!(WorkflowFile::parse("not json").unwrap_err().to_string().contains("not a workflow file"));
    }

    fn two_step(prompt: &str) -> WorkflowSpec {
        let mut spec = WorkflowSpec::blank("Daily");
        spec.steps.push(Step {
            id: "a".into(),
            title: "Ask".into(),
            x: 0,
            y: 0,
            kind: StepKind::Agent {
                agent: AgentSlug::new("researcher").unwrap(),
                prompt: prompt.into(),
                version: None,
                model: None,
                max_cost_micros: None,
            },
        });
        spec.edges.push(Edge { from: "start".into(), to: "a".into(), label: None });
        spec
    }

    #[test]
    fn the_outline_shows_settings_not_positions_and_a_diff_shows_only_what_changed() {
        let before = two_step("Look at the news.\nBe brief.");
        let mut moved = before.clone();
        moved.steps[1].x = 300;
        assert_eq!(outline(&before), outline(&moved));
        let text = outline(&before);
        assert!(text.contains("a · Ask (agent)"));
        assert!(text.contains("  prompt: Look at the news.\n    Be brief."));
        assert!(text.contains("start → a"));

        let after = two_step("Look at the news.\nBe thorough.");
        let diff = WorkflowService::diff(&before, &after);
        let changed: Vec<(&str, &str)> =
            diff.iter().filter(|l| l.kind != "same").map(|l| (l.kind, l.text.trim())).collect();
        assert_eq!(changed, vec![("removed", "Be brief."), ("added", "Be thorough.")]);
    }

    #[test]
    fn durations_read_in_the_largest_whole_unit() {
        assert_eq!(ms(2 * 3600 * 1000), "2 h");
        assert_eq!(ms(90 * 60 * 1000), "90 min");
        assert_eq!(ms(2 * 24 * 3600 * 1000), "2 d");
    }
}
