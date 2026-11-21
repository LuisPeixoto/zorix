//! What the screens, the commands and the manager's tools ask of projects (ADR-0036, ADR-0037).
//!
//! The service holds the rules: a task given to an agent or a team starts an ordinary run when it moves to In progress, a finished
//! run sends it to Review with the answer as a deliverable, a failed run says so, dependencies hold a run back, and a card leaves a
//! running lane only when its run is stopped. [`ProjectService::reconcile`] reads the runs of tasks and applies what they did; it is
//! safe to call at any time and as often as wanted.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use zorix_core::{
    AgentSlug, Clock, Date, Deliverable, DeliverableKind, Error, ExecutionId, NeedsYou, Project, ProjectEvent,
    ProjectEventDraft, ProjectEventKind, ProjectKey, ProjectStore, Result, RunOutcome, RunRequest, Task, TaskKey,
    TaskLink, TaskPriority, TaskRun, TaskState, TenantId, Timestamp, Who, check_dependencies,
};
use zorix_workflows::launcher::{Outcome, RunStart, StepLauncher, Target};

use crate::links::{PullReader, PullState};
use crate::rules::{LinkKind, RANK_STEP, answer_title, manager_prompt, rank_at, renumber, task_prompt};

/// The words an error starts with when a move would stop a run that is going, so a screen can ask first.
pub const RUN_GOING: &str = "run going:";

const ATTEMPTS: usize = 4;

fn invalid(what: impl Into<String>) -> Error {
    Error::InvalidInput(what.into())
}

/// A project to make.
#[derive(Clone, Debug, Deserialize)]
pub struct NewProject {
    /// Its key.
    pub key: ProjectKey,
    /// Its name.
    pub name: String,
    /// What it is for.
    #[serde(default)]
    pub description: String,
    /// The agent that plans it.
    #[serde(default)]
    pub manager: Option<AgentSlug>,
    /// First day.
    #[serde(default)]
    pub start: Option<Date>,
    /// Last day.
    #[serde(default)]
    pub due: Option<Date>,
    /// The most one task run may cost, in millionths of a dollar.
    #[serde(default)]
    pub cost_limit_micros: Option<u64>,
}

/// What a person may change about a project. All of it is replaced; a screen sends what it showed.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ProjectFields {
    /// Its name.
    pub name: String,
    /// What it is for.
    #[serde(default)]
    pub description: String,
    /// The agent that plans it.
    #[serde(default)]
    pub manager: Option<AgentSlug>,
    /// First day.
    #[serde(default)]
    pub start: Option<Date>,
    /// Last day.
    #[serde(default)]
    pub due: Option<Date>,
    /// The most one task run may cost.
    #[serde(default)]
    pub cost_limit_micros: Option<u64>,
    /// Hidden from the active list.
    #[serde(default)]
    pub archived: bool,
}

impl ProjectFields {
    /// What a project has now.
    pub fn of(project: &Project) -> Self {
        Self {
            name: project.name.clone(),
            description: project.description.clone(),
            manager: project.manager.clone(),
            start: project.start.clone(),
            due: project.due.clone(),
            cost_limit_micros: project.cost_limit_micros,
            archived: project.archived,
        }
    }
}

/// A task to make.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct NewTask {
    /// Its title.
    pub title: String,
    /// What to do.
    #[serde(default)]
    pub description: String,
    /// The lane it starts in (Backlog when absent).
    #[serde(default)]
    pub state: Option<TaskState>,
    /// How urgent.
    #[serde(default)]
    pub priority: TaskPriority,
    /// Who has it.
    #[serde(default)]
    pub assignee: Option<Who>,
    /// First day.
    #[serde(default)]
    pub start: Option<Date>,
    /// Last day.
    #[serde(default)]
    pub due: Option<Date>,
    /// What it waits for.
    #[serde(default)]
    pub depends_on: Vec<TaskKey>,
    /// Links.
    #[serde(default)]
    pub links: Vec<TaskLink>,
}

/// What a person may change about a task. All of it is replaced; a screen sends what it showed.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct TaskFields {
    /// Its title.
    pub title: String,
    /// What to do.
    #[serde(default)]
    pub description: String,
    /// How urgent.
    #[serde(default)]
    pub priority: TaskPriority,
    /// Who has it.
    #[serde(default)]
    pub assignee: Option<Who>,
    /// First day.
    #[serde(default)]
    pub start: Option<Date>,
    /// Last day.
    #[serde(default)]
    pub due: Option<Date>,
    /// What it waits for.
    #[serde(default)]
    pub depends_on: Vec<TaskKey>,
    /// Links.
    #[serde(default)]
    pub links: Vec<TaskLink>,
    /// 0 to 100.
    #[serde(default)]
    pub progress: u8,
}

impl TaskFields {
    /// What a task has now.
    pub fn of(task: &Task) -> Self {
        Self {
            title: task.title.clone(),
            description: task.description.clone(),
            priority: task.priority,
            assignee: task.assignee.clone(),
            start: task.start.clone(),
            due: task.due.clone(),
            depends_on: task.depends_on.clone(),
            links: task.links.clone(),
            progress: task.progress,
        }
    }
}

/// A move of a card.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Move {
    /// The lane to go to.
    pub to: Option<TaskState>,
    /// The place in that lane, from 0 (the end when absent).
    #[serde(default)]
    pub index: Option<usize>,
    /// What the person said when sending a task back. It reaches the run the task starts again.
    #[serde(default)]
    pub note: Option<String>,
    /// A person confirmed that a run that is going may be stopped.
    #[serde(default)]
    pub cancel_run: bool,
    /// Who approved the move when it needed approval (a manager's move to Done).
    #[serde(default)]
    pub approved_by: Option<String>,
}

/// A task as a screen shows it: the record and what is derived from it.
#[derive(Clone, Debug, Serialize)]
pub struct TaskView {
    /// The task.
    pub task: Task,
    /// The tasks it still waits for.
    pub blocked_by: Vec<TaskKey>,
    /// What a person must do about it.
    pub needs_you: Option<NeedsYou>,
    /// Whether an agent or a team is working on it now.
    pub working: bool,
}

fn view(task: Task, siblings: &[Task]) -> TaskView {
    let blocked_by = task.blocked_by(siblings);
    let needs_you = task.needs_you();
    let working = task.current_run().is_some_and(|r| r.outcome == RunOutcome::Running);
    TaskView { task, blocked_by, needs_you, working }
}

/// How many tasks are in each lane.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Counts {
    /// Not started.
    pub backlog: u32,
    /// Being worked on.
    pub in_progress: u32,
    /// Waiting for a person.
    pub review: u32,
    /// Finished.
    pub done: u32,
}

impl Counts {
    fn add(&mut self, state: TaskState, n: u32) {
        match state {
            TaskState::Backlog => self.backlog += n,
            TaskState::InProgress => self.in_progress += n,
            TaskState::Review => self.review += n,
            TaskState::Done => self.done += n,
        }
    }

    /// All tasks.
    pub fn total(&self) -> u32 {
        self.backlog + self.in_progress + self.review + self.done
    }
}

/// A project in the list.
#[derive(Clone, Debug, Serialize)]
pub struct ProjectCard {
    /// The project.
    pub project: Project,
    /// Its tasks by lane.
    pub counts: Counts,
    /// How many need a person.
    pub needs_you: u32,
    /// Who has tasks in it.
    pub assignees: Vec<Who>,
}

/// The manager's run, while it is going.
#[derive(Clone, Debug, Serialize)]
pub struct ManagerRun {
    /// The execution.
    pub execution: ExecutionId,
    /// Whether it is stopped for a person.
    pub waiting: bool,
}

/// A project with its tasks.
#[derive(Clone, Debug, Serialize)]
pub struct Board {
    /// The project.
    pub project: Project,
    /// Its tasks, by rank within each lane.
    pub tasks: Vec<TaskView>,
    /// The manager's run, when one is going.
    pub manager_run: Option<ManagerRun>,
}

/// One task with its history.
#[derive(Clone, Debug, Serialize)]
pub struct TaskDetail {
    /// The task.
    pub task: TaskView,
    /// What happened to it, newest first.
    pub events: Vec<ProjectEvent>,
}

/// One deliverable of the project, with its task.
#[derive(Clone, Debug, Serialize)]
pub struct DeliverableRow {
    /// The task.
    pub task: TaskKey,
    /// The task's title.
    pub title: String,
    /// The task's lane.
    pub state: TaskState,
    /// Who has the task.
    pub assignee: Option<Who>,
    /// The deliverable.
    pub deliverable: Deliverable,
}

/// A link with what ZORIX could read about it.
#[derive(Clone, Debug, Serialize)]
pub struct LinkState {
    /// The address.
    pub url: String,
    /// The state of the pull request it points at, when a connection could read it.
    pub pull: Option<PullState>,
}

/// Projects and tasks.
pub struct ProjectService {
    store: Arc<dyn ProjectStore>,
    launcher: Arc<dyn StepLauncher>,
    pulls: Option<Arc<dyn PullReader>>,
    tenant: TenantId,
    clock: Arc<dyn Clock>,
    gate: Mutex<()>,
    reads_runs: bool,
    model: parking_lot::Mutex<Option<String>>,
}

impl ProjectService {
    /// A service for one tenant.
    pub fn new(
        store: Arc<dyn ProjectStore>,
        launcher: Arc<dyn StepLauncher>,
        tenant: TenantId,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            store,
            launcher,
            pulls: None,
            tenant,
            clock,
            gate: Mutex::new(()),
            reads_runs: true,
            model: parking_lot::Mutex::new(None),
        }
    }

    /// Let the service read the state of pull requests on links.
    pub fn with_pulls(mut self, pulls: Arc<dyn PullReader>) -> Self {
        self.pulls = Some(pulls);
        self
    }

    /// A service that does not read runs when it is asked for projects and tasks: for a command that has no engine. What is stored
    /// is shown as it is, and whatever hosts the engine keeps it up to date.
    pub fn without_runs(mut self) -> Self {
        self.reads_runs = false;
        self
    }

    /// Run tasks with this model instead of their agent's own (a command's `--model`). None leaves it to the agent.
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

    fn draft(&self, task: Option<&TaskKey>, by: &Option<Who>, kind: ProjectEventKind) -> ProjectEventDraft {
        ProjectEventDraft { task: task.cloned(), by: by.clone(), kind, at: self.now() }
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Projects

    async fn project_of(&self, key: &ProjectKey) -> Result<Project> {
        self.store
            .project_by_key(&self.tenant, key)
            .await?
            .ok_or_else(|| Error::Missing(format!("There is no project {key}. `zorix project list` shows them.")))
    }

    /// One project.
    pub async fn project(&self, key: &ProjectKey) -> Result<Project> {
        self.project_of(key).await
    }

    /// The projects with their figures. Archived ones are left out unless asked for.
    pub async fn projects(&self, archived: bool) -> Result<Vec<ProjectCard>> {
        self.reconcile().await?;
        let mut cards = Vec::new();
        for project in self.store.projects(&self.tenant).await? {
            if project.archived != archived {
                continue;
            }
            let tasks = self.store.tasks(&self.tenant, project.id).await?;
            let mut counts = Counts::default();
            let mut needs_you = 0;
            let mut assignees: Vec<Who> = Vec::new();
            for task in &tasks {
                counts.add(task.state, 1);
                if task.needs_you().is_some() {
                    needs_you += 1;
                }
                if let Some(who) = &task.assignee
                    && !assignees.contains(who)
                {
                    assignees.push(who.clone());
                }
            }
            cards.push(ProjectCard { project, counts, needs_you, assignees });
        }
        Ok(cards)
    }

    /// Make a project.
    pub async fn create_project(&self, new: NewProject, by: Option<Who>) -> Result<Project> {
        let mut project = Project::new(self.tenant.clone(), new.key, new.name.trim(), self.now());
        project.description = new.description;
        project.manager = new.manager;
        project.start = new.start;
        project.due = new.due;
        project.cost_limit_micros = new.cost_limit_micros;
        self.store.create_project(&self.tenant, project, by.or(Some(Who::You))).await
    }

    /// Change a project.
    pub async fn update_project(&self, key: &ProjectKey, fields: ProjectFields, by: Option<Who>) -> Result<Project> {
        let mut project = self.project_of(key).await?;
        let was_archived = project.archived;
        let changed = ProjectFields::of(&project) != fields;
        project.name = fields.name.trim().to_string();
        project.description = fields.description;
        project.manager = fields.manager;
        project.start = fields.start;
        project.due = fields.due;
        project.cost_limit_micros = fields.cost_limit_micros;
        project.archived = fields.archived;
        project.updated_at = self.now();
        let mut events = Vec::new();
        if changed {
            events.push(self.draft(None, &by, ProjectEventKind::ProjectEdited));
        }
        if was_archived != project.archived {
            events.push(self.draft(None, &by, ProjectEventKind::ProjectArchived { archived: project.archived }));
        }
        self.store.update_project(&self.tenant, &project, &events).await
    }

    /// Delete a project with its tasks and history. Runs that tasks started are left as they are.
    pub async fn remove_project(&self, key: &ProjectKey) -> Result<()> {
        let project = self.project_of(key).await?;
        for task in self.store.tasks(&self.tenant, project.id).await? {
            if let Some(run) = task.current_run() {
                let _ = self.launcher.cancel(run.execution, "The project was deleted.").await;
            }
        }
        self.store.remove_project(&self.tenant, project.id).await?;
        Ok(())
    }

    /// A project with its tasks, after the runs of tasks were read.
    pub async fn board(&self, key: &ProjectKey) -> Result<Board> {
        self.reconcile().await?;
        let project = self.project_of(key).await?;
        let tasks = self.store.tasks(&self.tenant, project.id).await?;
        let views = tasks.iter().cloned().map(|t| view(t, &tasks)).collect();
        let manager_run = self.manager_run(&project).await;
        Ok(Board { project, tasks: views, manager_run })
    }

    async fn manager_run(&self, project: &Project) -> Option<ManagerRun> {
        project.manager.as_ref()?;
        let events = self.store.events(&self.tenant, project.id, None, 30).await.ok()?;
        let execution = events.iter().find_map(|e| match e.kind {
            ProjectEventKind::ManagerAsked { execution } => Some(execution),
            _ => None,
        })?;
        match self.launcher.outcome(execution).await.ok()? {
            Outcome::Running { .. } => Some(ManagerRun { execution, waiting: false }),
            Outcome::WaitingForPerson { .. } => Some(ManagerRun { execution, waiting: true }),
            _ => None,
        }
    }

    /// Ask the project's manager to plan it. Returns the run.
    pub async fn ask_manager(&self, key: &ProjectKey, ask: Option<&str>) -> Result<ExecutionId> {
        let project = self.project_of(key).await?;
        if project.archived {
            return Err(invalid(format!("{key} is archived.")));
        }
        let Some(manager) = project.manager.clone() else {
            return Err(invalid(format!("{key} has no manager. Choose one in the project's settings.")));
        };
        let now = self.now();
        let start = RunStart {
            target: Target::Agent { agent: manager.clone(), version: None },
            prompt: manager_prompt(&project, ask),
            input: None,
            model: self.model.lock().clone(),
            max_cost_micros: project.cost_limit_micros,
            key: format!("pm-{}-{}", project.id, now.millis()),
            metadata: BTreeMap::from([
                ("project".to_string(), project.key.to_string()),
                ("project_manager".to_string(), "1".to_string()),
            ]),
            access: vec![format!("project:{}:write", project.key)],
        };
        let execution = self.launcher.start(start).await.map_err(invalid)?;
        let by = Some(Who::You);
        self.store
            .add_events(
                &self.tenant,
                project.id,
                &[self.draft(None, &by, ProjectEventKind::ManagerAsked { execution })],
            )
            .await?;
        Ok(execution)
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Tasks

    async fn load(&self, key: &TaskKey) -> Result<(Task, Project, Vec<Task>)> {
        let task = self
            .store
            .task(&self.tenant, key)
            .await?
            .ok_or_else(|| Error::Missing(format!("There is no task {key}. `zorix task list` shows them.")))?;
        let project = self
            .store
            .project(&self.tenant, task.project)
            .await?
            .ok_or_else(|| Error::Missing(format!("The project of {key} is gone.")))?;
        let siblings = self.store.tasks(&self.tenant, project.id).await?;
        Ok((task, project, siblings))
    }

    /// Read a task and change it with `change`, retrying when somebody else wrote it first. `change` returns the events to record,
    /// or none when there is nothing to write.
    async fn update<F>(&self, key: &TaskKey, change: F) -> Result<Task>
    where
        F: Fn(&mut Task, &Project, &[Task], Timestamp) -> Result<Option<Vec<ProjectEventDraft>>>,
    {
        for _ in 0..ATTEMPTS {
            let (mut task, project, siblings) = self.load(key).await?;
            if project.archived {
                return Err(invalid(format!("{} is archived. Bring it back to change its tasks.", project.key)));
            }
            let expected = task.revision;
            let now = self.now();
            let Some(events) = change(&mut task, &project, &siblings, now)? else {
                return Ok(task);
            };
            task.updated_at = now;
            task.validate()?;
            if self.store.put_task(&self.tenant, &task, Some(expected), &events).await? {
                task.revision = expected + 1;
                return Ok(task);
            }
        }
        Err(invalid(format!("{key} was changed by something else at the same time. Try again.")))
    }

    /// One task with its history.
    pub async fn task(&self, key: &TaskKey) -> Result<TaskDetail> {
        self.reconcile().await?;
        let (task, project, siblings) = self.load(key).await?;
        let events = self.store.events(&self.tenant, project.id, Some(key), 200).await?;
        Ok(TaskDetail { task: view(task, &siblings), events })
    }

    /// The tasks of a project with their figures, in board order.
    pub async fn tasks(&self, key: &ProjectKey) -> Result<Vec<TaskView>> {
        Ok(self.board(key).await?.tasks)
    }

    /// Make a task.
    pub async fn add_task(&self, project: &ProjectKey, new: NewTask, by: Option<Who>) -> Result<Task> {
        let project = self.project_of(project).await?;
        if project.archived {
            return Err(invalid(format!("{} is archived. Bring it back to add tasks.", project.key)));
        }
        let siblings = self.store.tasks(&self.tenant, project.id).await?;
        let number = self.store.next_task_number(&self.tenant, project.id).await?;
        let key = TaskKey::new(project.key.clone(), number).map_err(invalid)?;
        let now = self.now();
        let mut task = Task::new(self.tenant.clone(), project.id, key.clone(), new.title.trim(), now);
        task.description = new.description;
        task.state = new.state.unwrap_or(TaskState::Backlog);
        task.priority = new.priority;
        task.assignee = new.assignee;
        task.start = new.start;
        task.due = new.due;
        task.depends_on = new.depends_on;
        task.links = new.links;
        task.rank = siblings.iter().filter(|t| t.state == task.state).map(|t| t.rank).max().unwrap_or(0) + RANK_STEP;
        if task.state == TaskState::Done {
            task.progress = 100;
        }
        if task.state == TaskState::InProgress && task.assignee.as_ref().is_some_and(Who::runs) {
            task.run_requested = Some(RunRequest::default());
        }
        task.validate()?;
        check_dependencies(&key, &task.depends_on, &siblings)?;
        let by = by.or(Some(Who::You));
        let mut events = vec![self.draft(Some(&key), &by, ProjectEventKind::TaskCreated { title: task.title.clone() })];
        if let Some(who) = &task.assignee {
            events.push(self.draft(Some(&key), &by, ProjectEventKind::Assigned { to: Some(who.clone()) }));
        }
        if !self.store.put_task(&self.tenant, &task, None, &events).await? {
            return Err(invalid(format!("{key} already exists.")));
        }
        if task.run_requested.is_some() {
            self.start_ready(&key).await?;
        }
        Ok(self.store.task(&self.tenant, &key).await?.unwrap_or(task))
    }

    /// Change a task's details.
    pub async fn update_task(&self, key: &TaskKey, fields: TaskFields, by: Option<Who>) -> Result<Task> {
        let by = by.or(Some(Who::You));
        let task = self
            .update(key, |task, _project, siblings, _now| {
                let before = TaskFields::of(task);
                if before == fields {
                    return Ok(None);
                }
                if before.assignee != fields.assignee && task.current_run().is_some() {
                    return Err(invalid(format!(
                        "{key} has a run going. Move it out of In progress to stop it before giving it to someone else."
                    )));
                }
                check_dependencies(key, &fields.depends_on, siblings)?;
                let mut events = Vec::new();
                if before.assignee != fields.assignee {
                    events.push(self.draft(Some(key), &by, ProjectEventKind::Assigned { to: fields.assignee.clone() }));
                    if task.state == TaskState::InProgress && fields.assignee.as_ref().is_some_and(Who::runs) {
                        task.run_requested = Some(RunRequest::default());
                    } else {
                        task.run_requested = None;
                    }
                }
                for link in fields.links.iter().filter(|l| !before.links.contains(l)) {
                    events.push(self.draft(Some(key), &by, ProjectEventKind::Linked { url: link.url.clone() }));
                }
                let rest_changed = (
                    &before.title,
                    &before.description,
                    before.priority,
                    &before.start,
                    &before.due,
                    &before.depends_on,
                    before.progress,
                ) != (
                    &fields.title,
                    &fields.description,
                    fields.priority,
                    &fields.start,
                    &fields.due,
                    &fields.depends_on,
                    fields.progress,
                );
                if rest_changed {
                    events.push(self.draft(Some(key), &by, ProjectEventKind::TaskEdited));
                }
                task.title = fields.title.trim().to_string();
                task.description = fields.description.clone();
                task.priority = fields.priority;
                task.assignee = fields.assignee.clone();
                task.start = fields.start.clone();
                task.due = fields.due.clone();
                task.depends_on = fields.depends_on.clone();
                task.links = fields.links.clone();
                task.progress = fields.progress;
                Ok(Some(events))
            })
            .await?;
        if task.run_requested.is_some() {
            self.start_ready(key).await?;
            return Ok(self.store.task(&self.tenant, key).await?.unwrap_or(task));
        }
        Ok(task)
    }

    /// Move a card to a lane and a place in it. A move to In progress starts the run of an agent's or a team's task; a move out of
    /// In progress while a run is going stops the run, and says so first unless `cancel_run` is set.
    pub async fn move_task(&self, key: &TaskKey, mv: Move, by: Option<Who>) -> Result<Task> {
        let (task, project, siblings) = self.load(key).await?;
        if project.archived {
            return Err(invalid(format!("{} is archived. Bring it back to change its tasks.", project.key)));
        }
        let to = mv.to.unwrap_or(task.state);
        if to == task.state && mv.index.is_none() {
            return Ok(task);
        }
        let leaving = task.state == TaskState::InProgress && to != TaskState::InProgress;
        let running = if leaving { task.current_run().cloned() } else { None };
        if running.is_some() && !mv.cancel_run {
            return Err(invalid(format!(
                "{RUN_GOING} {key} has a run going. Moving it out of In progress stops the run."
            )));
        }
        let rank = self.place(&siblings, key, to, mv.index).await?;
        if let Some(run) = &running {
            let _ = self.launcher.cancel(run.execution, "The task was moved out of In progress.").await;
        }
        let by = by.or(Some(Who::You));
        let moved = self
            .update(key, |task, _project, siblings, now| {
                let from = task.state;
                let mut events = Vec::new();
                task.state = to;
                task.rank = rank;
                if to == TaskState::Done {
                    task.progress = 100;
                }
                if from != to {
                    events.push(self.draft(
                        Some(key),
                        &by,
                        ProjectEventKind::Moved {
                            from,
                            to,
                            approved_by: mv.approved_by.clone(),
                            note: mv.note.clone().filter(|n| !n.trim().is_empty()),
                        },
                    ));
                }
                if from == TaskState::InProgress && to != TaskState::InProgress {
                    task.run_requested = None;
                    if let Some(run) = task.runs.last_mut()
                        && !run.outcome.ended()
                    {
                        run.outcome = RunOutcome::Cancelled;
                        run.ended_at = Some(now);
                        events.push(self.draft(
                            Some(key),
                            &by,
                            ProjectEventKind::RunCancelled { execution: run.execution },
                        ));
                    }
                }
                if to == TaskState::InProgress
                    && from != TaskState::InProgress
                    && task.assignee.as_ref().is_some_and(Who::runs)
                {
                    task.run_requested =
                        Some(RunRequest { note: mv.note.clone().filter(|n| !n.trim().is_empty()), error: None });
                    let blocked = task.blocked_by(siblings);
                    if !blocked.is_empty() {
                        events.push(self.draft(Some(key), &by, ProjectEventKind::Blocked { by: blocked }));
                    }
                }
                Ok(Some(events))
            })
            .await?;
        if moved.run_requested.is_some() {
            self.start_ready(key).await?;
            return Ok(self.store.task(&self.tenant, key).await?.unwrap_or(moved));
        }
        Ok(moved)
    }

    /// Run a task's agent again after a failure, or without a note: the task goes back to waiting for a run.
    pub async fn restart_task(&self, key: &TaskKey, note: Option<String>, by: Option<Who>) -> Result<Task> {
        let _ = by;
        let task = self
            .update(key, |task, _project, _siblings, _now| {
                if task.state != TaskState::InProgress {
                    return Err(invalid(format!("{key} is not in progress. Move it there to run it.")));
                }
                if !task.assignee.as_ref().is_some_and(Who::runs) {
                    return Err(invalid(format!(
                        "{key} is not given to an agent or a team, so there is nothing to run."
                    )));
                }
                if task.current_run().is_some() {
                    return Err(invalid(format!("{key} already has a run going.")));
                }
                task.run_requested =
                    Some(RunRequest { note: note.clone().filter(|n| !n.trim().is_empty()), error: None });
                Ok(Some(Vec::new()))
            })
            .await?;
        self.start_ready(key).await?;
        Ok(self.store.task(&self.tenant, key).await?.unwrap_or(task))
    }

    /// The rank for a card placed at `index` in lane `to`. When the lane has no room it is numbered again first.
    async fn place(&self, siblings: &[Task], key: &TaskKey, to: TaskState, index: Option<usize>) -> Result<i64> {
        let mut lane: Vec<&Task> = siblings.iter().filter(|t| t.state == to && &t.key != key).collect();
        lane.sort_by_key(|t| (t.rank, t.key.number));
        let index = index.unwrap_or(lane.len()).min(lane.len());
        let ranks: Vec<i64> = lane.iter().map(|t| t.rank).collect();
        if let Some(rank) = rank_at(&ranks, index) {
            return Ok(rank);
        }
        let numbers = renumber(lane.len() + 1);
        for (slot, task) in lane.iter().enumerate() {
            let rank = numbers[if slot < index { slot } else { slot + 1 }];
            if task.rank != rank {
                let mut next = (*task).clone();
                next.rank = rank;
                next.updated_at = self.now();
                let _ = self.store.put_task(&self.tenant, &next, Some(task.revision), &[]).await?;
            }
        }
        Ok(numbers[index])
    }

    /// Delete a task. A run it has going is stopped.
    pub async fn remove_task(&self, key: &TaskKey) -> Result<()> {
        let (task, _project, _siblings) = self.load(key).await?;
        if let Some(run) = task.current_run() {
            let _ = self.launcher.cancel(run.execution, "The task was deleted.").await;
        }
        self.store.remove_task(&self.tenant, key).await?;
        Ok(())
    }

    /// Say something on a task.
    pub async fn comment(&self, key: &TaskKey, text: &str, by: Option<Who>) -> Result<()> {
        let text = text.trim();
        if text.is_empty() {
            return Err(invalid("A comment needs some text."));
        }
        if text.chars().count() > 4_000 {
            return Err(invalid("A comment is at most 4000 characters."));
        }
        let by = by.or(Some(Who::You));
        self.update(key, |_task, _project, _siblings, _now| {
            Ok(Some(vec![self.draft(Some(key), &by, ProjectEventKind::Comment { text: text.to_string() })]))
        })
        .await?;
        Ok(())
    }

    /// Put a link on a task.
    pub async fn add_link(&self, key: &TaskKey, url: &str, title: Option<String>, by: Option<Who>) -> Result<Task> {
        let by = by.or(Some(Who::You));
        let link = TaskLink { url: url.trim().to_string(), title: title.filter(|t| !t.trim().is_empty()) };
        self.update(key, |task, _project, _siblings, _now| {
            if task.links.iter().any(|l| l.url == link.url) {
                return Ok(None);
            }
            task.links.push(link.clone());
            Ok(Some(vec![self.draft(Some(key), &by, ProjectEventKind::Linked { url: link.url.clone() })]))
        })
        .await
    }

    /// Add something a task produced.
    pub async fn add_deliverable(
        &self,
        key: &TaskKey,
        kind: DeliverableKind,
        title: &str,
        body: &str,
        by: Option<Who>,
        execution: Option<ExecutionId>,
    ) -> Result<Task> {
        let (title, body) = (title.trim().to_string(), body.trim().to_string());
        if title.is_empty() || body.is_empty() {
            return Err(invalid("A deliverable needs a title and something to show."));
        }
        if body.chars().count() > 200_000 {
            return Err(invalid("A deliverable is at most 200000 characters."));
        }
        if kind == DeliverableKind::Link && !(body.starts_with("https://") || body.starts_with("http://")) {
            return Err(invalid("A link deliverable is an http or https address."));
        }
        self.update(key, |task, _project, _siblings, now| {
            let n = task.deliverables.len() + 1;
            task.deliverables.push(Deliverable {
                id: format!("d{n}"),
                kind,
                title: title.clone(),
                body: body.clone(),
                by: by.clone(),
                at: now,
                execution,
            });
            Ok(Some(vec![self.draft(Some(key), &by, ProjectEventKind::Deliverable { title: title.clone() })]))
        })
        .await
    }

    /// Say how far a task is.
    pub async fn report_progress(&self, key: &TaskKey, percent: u8, by: Option<Who>) -> Result<Task> {
        if percent > 100 {
            return Err(invalid("Progress is from 0 to 100."));
        }
        self.update(key, |task, _project, _siblings, _now| {
            if task.progress == percent {
                return Ok(None);
            }
            task.progress = percent;
            Ok(Some(vec![self.draft(Some(key), &by, ProjectEventKind::Progress { percent })]))
        })
        .await
    }

    /// Every deliverable of a project, newest first.
    pub async fn deliverables(&self, key: &ProjectKey) -> Result<Vec<DeliverableRow>> {
        let project = self.project_of(key).await?;
        let mut rows = Vec::new();
        for task in self.store.tasks(&self.tenant, project.id).await? {
            for deliverable in &task.deliverables {
                rows.push(DeliverableRow {
                    task: task.key.clone(),
                    title: task.title.clone(),
                    state: task.state,
                    assignee: task.assignee.clone(),
                    deliverable: deliverable.clone(),
                });
            }
        }
        rows.sort_by_key(|row| std::cmp::Reverse(row.deliverable.at));
        Ok(rows)
    }

    /// A project's history, newest first, of one task or of all.
    pub async fn history(
        &self,
        project: &ProjectKey,
        task: Option<&TaskKey>,
        limit: usize,
    ) -> Result<Vec<ProjectEvent>> {
        let project = self.project_of(project).await?;
        self.store.events(&self.tenant, project.id, task, limit).await
    }

    /// What ZORIX can read about the links of a task: the state of the pull requests among them.
    pub async fn link_states(&self, key: &TaskKey) -> Result<Vec<LinkState>> {
        let (task, _project, _siblings) = self.load(key).await?;
        let mut out = Vec::new();
        for link in &task.links {
            let kind = LinkKind::of(&link.url);
            let pull = match (&self.pulls, &kind) {
                (Some(reader), LinkKind::GithubPull { .. } | LinkKind::GitlabPull { .. }) => reader.read(&kind).await,
                _ => None,
            };
            out.push(LinkState { url: link.url.clone(), pull });
        }
        Ok(out)
    }

    // -----------------------------------------------------------------------------------------------------------------
    // Runs

    /// Start the run a task waits for, when it can start: it is in progress, nothing is going, and what it depends on is done.
    /// Returns whether a run started.
    pub async fn start_ready(&self, key: &TaskKey) -> Result<bool> {
        let (task, project, siblings) = self.load(key).await?;
        if task.state != TaskState::InProgress || task.current_run().is_some() || project.archived {
            return Ok(false);
        }
        let Some(request) = task.run_requested.clone() else {
            return Ok(false);
        };
        if request.error.is_some() || !task.blocked_by(&siblings).is_empty() {
            return Ok(false);
        }
        let target = match &task.assignee {
            Some(Who::Agent { slug }) => Target::Agent { agent: slug.clone(), version: None },
            Some(Who::Team { slug }) => Target::Team { team: slug.clone() },
            _ => return Ok(false),
        };
        let start = RunStart {
            target,
            prompt: task_prompt(&project, &task, &siblings, request.note.as_deref()),
            input: None,
            model: self.model.lock().clone(),
            max_cost_micros: project.cost_limit_micros,
            key: format!("task-{}-{}", task.id, task.runs.len()),
            metadata: BTreeMap::from([
                ("project".to_string(), project.key.to_string()),
                ("task".to_string(), task.key.to_string()),
            ]),
            access: vec![format!("task:{}", task.key)],
        };
        let assignee = task.assignee.clone();
        match self.launcher.start(start).await {
            Ok(execution) => {
                let note = request.note.clone();
                self.update(key, |task, _project, _siblings, now| {
                    if task.state != TaskState::InProgress
                        || task.run_requested.is_none()
                        || task.current_run().is_some()
                    {
                        return Ok(None);
                    }
                    task.run_requested = None;
                    task.runs.push(TaskRun {
                        execution,
                        started_at: now,
                        ended_at: None,
                        outcome: RunOutcome::Running,
                        note: note.clone(),
                        reason: None,
                        cost_micros: 0,
                    });
                    let keep = zorix_core::MAX_TASK_RUNS;
                    if task.runs.len() > keep {
                        task.runs.drain(..task.runs.len() - keep);
                    }
                    Ok(Some(vec![self.draft(Some(key), &assignee, ProjectEventKind::RunStarted { execution })]))
                })
                .await?;
                Ok(true)
            }
            Err(reason) => {
                self.update(key, |task, _project, _siblings, _now| {
                    let Some(request) = task.run_requested.as_mut() else {
                        return Ok(None);
                    };
                    request.error = Some(reason.clone());
                    Ok(Some(vec![self.draft(
                        Some(key),
                        &None,
                        ProjectEventKind::StartFailed { reason: reason.clone() },
                    )]))
                })
                .await?;
                Ok(false)
            }
        }
    }

    /// Read the runs of tasks and apply what they did, and start the runs that were waiting for something. Returns how many tasks
    /// changed. It never fails because of one task.
    pub async fn reconcile(&self) -> Result<usize> {
        if !self.reads_runs {
            return Ok(0);
        }
        let _turn = self.gate.lock().await;
        let mut changed = 0;
        for task in self.store.tasks_in_state(&self.tenant, TaskState::InProgress).await? {
            let key = task.key.clone();
            match self.settle(task).await {
                Ok(true) => changed += 1,
                Ok(false) => {}
                Err(error) => tracing::warn!(task = %key, %error, "a task could not be brought up to date"),
            }
        }
        Ok(changed)
    }

    async fn settle(&self, task: Task) -> Result<bool> {
        let key = task.key.clone();
        if let Some(run) = task.current_run().cloned() {
            let outcome = match self.launcher.outcome(run.execution).await {
                Ok(outcome) => outcome,
                Err(error) => {
                    tracing::warn!(task = %key, %error, "the run of a task could not be read");
                    return Ok(false);
                }
            };
            return self.apply(&key, run.execution, outcome).await;
        }
        if task.run_requested.as_ref().is_some_and(|r| r.error.is_none()) {
            return self.start_ready(&key).await;
        }
        Ok(false)
    }

    async fn apply(&self, key: &TaskKey, execution: ExecutionId, outcome: Outcome) -> Result<bool> {
        let before = self.store.task(&self.tenant, key).await?.map(|t| t.revision);
        let assignee_events = |task: &Task| task.assignee.clone();
        let task = self
            .update(key, |task, _project, siblings, now| {
                let who = assignee_events(task);
                let Some(index) = task.runs.iter().position(|r| r.execution == execution && !r.outcome.ended()) else {
                    return Ok(None);
                };
                match &outcome {
                    Outcome::Running { cost_micros } => {
                        let run = &mut task.runs[index];
                        if run.outcome == RunOutcome::Running && run.cost_micros == *cost_micros {
                            return Ok(None);
                        }
                        run.outcome = RunOutcome::Running;
                        run.cost_micros = *cost_micros;
                        Ok(Some(Vec::new()))
                    }
                    Outcome::WaitingForPerson { cost_micros } => {
                        let run = &mut task.runs[index];
                        if run.outcome == RunOutcome::Waiting && run.cost_micros == *cost_micros {
                            return Ok(None);
                        }
                        run.outcome = RunOutcome::Waiting;
                        run.cost_micros = *cost_micros;
                        Ok(Some(Vec::new()))
                    }
                    Outcome::Completed { text, cost_micros, .. } => {
                        let run = &mut task.runs[index];
                        run.outcome = RunOutcome::Finished;
                        run.ended_at = Some(now);
                        run.cost_micros = *cost_micros;
                        let mut events =
                            vec![self.draft(Some(key), &None, ProjectEventKind::RunFinished { execution })];
                        if !text.trim().is_empty() {
                            let title = answer_title(&key.to_string(), text);
                            let n = task.deliverables.len() + 1;
                            task.deliverables.push(Deliverable {
                                id: format!("d{n}"),
                                kind: DeliverableKind::Answer,
                                title: title.clone(),
                                body: text.trim().to_string(),
                                by: who.clone(),
                                at: now,
                                execution: Some(execution),
                            });
                            events.push(self.draft(Some(key), &who, ProjectEventKind::Deliverable { title }));
                        }
                        if task.state == TaskState::InProgress {
                            task.progress = 100;
                            task.state = TaskState::Review;
                            task.rank = siblings
                                .iter()
                                .filter(|t| t.state == TaskState::Review)
                                .map(|t| t.rank)
                                .max()
                                .unwrap_or(0)
                                + RANK_STEP;
                            events.push(self.draft(
                                Some(key),
                                &None,
                                ProjectEventKind::Moved {
                                    from: TaskState::InProgress,
                                    to: TaskState::Review,
                                    approved_by: None,
                                    note: None,
                                },
                            ));
                        }
                        Ok(Some(events))
                    }
                    Outcome::Failed { reason, cost_micros } => {
                        let run = &mut task.runs[index];
                        run.outcome = RunOutcome::Failed;
                        run.ended_at = Some(now);
                        run.cost_micros = *cost_micros;
                        run.reason = Some(reason.clone());
                        Ok(Some(vec![self.draft(
                            Some(key),
                            &None,
                            ProjectEventKind::RunFailed { execution, reason: reason.clone() },
                        )]))
                    }
                    Outcome::Cancelled => {
                        let run = &mut task.runs[index];
                        run.outcome = RunOutcome::Cancelled;
                        run.ended_at = Some(now);
                        Ok(Some(vec![self.draft(Some(key), &None, ProjectEventKind::RunCancelled { execution })]))
                    }
                }
            })
            .await?;
        Ok(before != Some(task.revision))
    }
}
