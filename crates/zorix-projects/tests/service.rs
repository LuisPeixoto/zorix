//! Projects end to end with a scripted launcher and a manual clock: a move starts a run, a finished run sends the task to Review with
//! its answer, a failed run needs a person, dependencies hold a run back, a move that would stop a run says so first, and the
//! manager's tools ask while a task's own tools reach only that task.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zorix_core::{
    Action, AgentBinding, AgentId, AgentSlug, ExecutionId, ManualClock, NeedsYou, Permission, ProjectEventKind,
    ProjectKey, RunOutcome, TaskKey, TaskState, TenantId, Timestamp, Tool, ToolCallId, ToolContext, Who, access,
};
use zorix_projects::{Move, NewProject, NewTask, ProjectService, RUN_GOING, TaskFields, project_tools};
use zorix_store::SqliteProjectStore;
use zorix_workflows::launcher::{Outcome, RunStart, StepLauncher, Target, ToolCall};

#[derive(Default)]
struct World {
    started: Vec<(ExecutionId, RunStart)>,
    outcomes: HashMap<ExecutionId, Outcome>,
    cancelled: Vec<ExecutionId>,
    start_error: Option<String>,
}

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<World>>);

impl Fake {
    fn set(&self, index: usize, outcome: Outcome) {
        let mut world = self.0.lock();
        let id = world.started[index].0;
        world.outcomes.insert(id, outcome);
    }

    fn finish(&self, index: usize, text: &str) {
        self.set(index, Outcome::Completed { text: text.into(), structured: None, cost_micros: 420_000, steps: 3 });
    }

    fn started(&self) -> usize {
        self.0.lock().started.len()
    }

    fn start_of(&self, index: usize) -> RunStart {
        self.0.lock().started[index].1.clone()
    }
}

#[async_trait]
impl StepLauncher for Fake {
    async fn start(&self, start: RunStart) -> Result<ExecutionId, String> {
        let mut world = self.0.lock();
        if let Some(error) = &world.start_error {
            return Err(error.clone());
        }
        if let Some((id, _)) = world.started.iter().find(|(_, s)| s.key == start.key) {
            return Ok(*id);
        }
        let id = ExecutionId::new();
        world.outcomes.insert(id, Outcome::Running { cost_micros: 0 });
        world.started.push((id, start));
        Ok(id)
    }

    async fn outcome(&self, id: ExecutionId) -> Result<Outcome, String> {
        self.0.lock().outcomes.get(&id).cloned().ok_or_else(|| "unknown execution".into())
    }

    async fn cancel(&self, id: ExecutionId, _reason: &str) -> Result<(), String> {
        let mut world = self.0.lock();
        world.cancelled.push(id);
        world.outcomes.insert(id, Outcome::Cancelled);
        Ok(())
    }

    async fn call_tool(&self, _call: &ToolCall) -> Result<Value, String> {
        Ok(json!({}))
    }
}

struct Rig {
    service: Arc<ProjectService>,
    fake: Fake,
}

fn rig() -> Rig {
    let store = Arc::new(SqliteProjectStore::open_in_memory().unwrap());
    let fake = Fake::default();
    let clock = Arc::new(ManualClock::new(Timestamp(1_700_000_000_000)));
    let service = Arc::new(ProjectService::new(store, Arc::new(fake.clone()), TenantId::local(), clock));
    Rig { service, fake }
}

fn atlas() -> AgentSlug {
    AgentSlug::new("atlas").unwrap()
}

fn scout() -> Who {
    Who::Agent { slug: AgentSlug::new("scout").unwrap() }
}

async fn project(rig: &Rig, key: &str) -> ProjectKey {
    let key = ProjectKey::new(key).unwrap();
    rig.service
        .create_project(
            NewProject {
                key: key.clone(),
                name: format!("Project {key}"),
                description: "Keep incidents rare.".into(),
                manager: Some(atlas()),
                start: None,
                due: None,
                cost_limit_micros: Some(1_000_000),
            },
            None,
        )
        .await
        .unwrap();
    key
}

async fn task(rig: &Rig, project: &ProjectKey, title: &str, who: Option<Who>) -> TaskKey {
    rig.service
        .add_task(project, NewTask { title: title.into(), assignee: who, ..NewTask::default() }, None)
        .await
        .unwrap()
        .key
}

fn to(state: TaskState) -> Move {
    Move { to: Some(state), ..Move::default() }
}

#[tokio::test]
async fn moving_an_agents_task_to_in_progress_starts_a_run_and_a_finished_run_sends_it_to_review() {
    let rig = rig();
    let ops = project(&rig, "OPS").await;
    let key = task(&rig, &ops, "Triage the backlog", Some(scout())).await;
    assert_eq!(rig.fake.started(), 0, "a task in the backlog starts nothing");

    let moved = rig.service.move_task(&key, to(TaskState::InProgress), None).await.unwrap();
    assert_eq!(moved.state, TaskState::InProgress);
    assert_eq!(rig.fake.started(), 1);
    let start = rig.fake.start_of(0);
    assert_eq!(start.target, Target::Agent { agent: AgentSlug::new("scout").unwrap(), version: None });
    assert_eq!(start.metadata.get("task").map(String::as_str), Some("OPS-1"));
    assert_eq!(start.metadata.get("project").map(String::as_str), Some("OPS"));
    assert_eq!(start.access, vec!["task:OPS-1".to_string()]);
    assert_eq!(start.max_cost_micros, Some(1_000_000), "the project's limit narrows the run");
    assert!(start.prompt.contains("OPS-1: <data source=\"task.title\" untrusted=\"true\">Triage the backlog</data>"));
    let detail = rig.service.task(&key).await.unwrap();
    assert_eq!(detail.task.task.runs.len(), 1);
    assert!(detail.task.working && detail.task.needs_you.is_none());

    rig.fake.finish(0, "# Backlog sorted\n12 items, 3 urgent.");
    let detail = rig.service.task(&key).await.unwrap();
    let t = &detail.task.task;
    assert_eq!((t.state, t.progress), (TaskState::Review, 100));
    assert_eq!(t.runs[0].outcome, RunOutcome::Finished);
    assert_eq!(t.runs[0].cost_micros, 420_000);
    assert_eq!(t.deliverables.len(), 1);
    assert_eq!(t.deliverables[0].title, "Backlog sorted");
    assert!(t.deliverables[0].body.contains("3 urgent"));
    assert_eq!(detail.task.needs_you, Some(NeedsYou::Review));
    let kinds: Vec<_> = detail.events.iter().map(|e| e.kind.clone()).collect();
    assert!(kinds.iter().any(|k| matches!(k, ProjectEventKind::RunStarted { .. })));
    assert!(kinds.iter().any(|k| matches!(k, ProjectEventKind::Moved { to: TaskState::Review, .. })));
    assert_eq!(rig.fake.started(), 1, "reading the task again does not start another run");
}

#[tokio::test]
async fn a_task_that_you_have_starts_no_run() {
    let rig = rig();
    let ops = project(&rig, "OPS").await;
    let key = task(&rig, &ops, "Decide the retention policy", Some(Who::You)).await;
    rig.service.move_task(&key, to(TaskState::InProgress), None).await.unwrap();
    assert_eq!(rig.fake.started(), 0);
    let unassigned = task(&rig, &ops, "Think about it", None).await;
    rig.service.move_task(&unassigned, to(TaskState::InProgress), None).await.unwrap();
    assert_eq!(rig.fake.started(), 0);
}

#[tokio::test]
async fn a_task_remembers_the_day_it_went_to_done_and_forgets_it_when_it_leaves() {
    let rig = rig();
    let ops = project(&rig, "OPS").await;
    let key = task(&rig, &ops, "Decide the retention policy", Some(Who::You)).await;
    assert!(rig.service.task(&key).await.unwrap().task.task.done_at.is_none());
    rig.service.move_task(&key, to(TaskState::Done), None).await.unwrap();
    let done = rig.service.task(&key).await.unwrap().task.task;
    assert!(done.done_at.is_some(), "the move to Done is remembered");
    rig.service.move_task(&key, to(TaskState::Review), None).await.unwrap();
    assert!(rig.service.task(&key).await.unwrap().task.task.done_at.is_none(), "and forgotten when it goes back");
}

#[tokio::test]
async fn a_failed_run_stays_in_progress_needs_you_and_can_be_run_again() {
    let rig = rig();
    let ops = project(&rig, "OPS").await;
    let key = task(&rig, &ops, "Rotate the key", Some(scout())).await;
    rig.service.move_task(&key, to(TaskState::InProgress), None).await.unwrap();
    rig.fake.set(0, Outcome::Failed { reason: "The provider refused the key.".into(), cost_micros: 5 });
    let detail = rig.service.task(&key).await.unwrap();
    assert_eq!(detail.task.task.state, TaskState::InProgress);
    assert_eq!(detail.task.needs_you, Some(NeedsYou::RunFailed));
    assert_eq!(detail.task.task.runs[0].reason.as_deref(), Some("The provider refused the key."));
    assert_eq!(rig.fake.started(), 1, "a failure is not retried by itself");

    rig.service.restart_task(&key, Some("Try the other provider.".into()), None).await.unwrap();
    assert_eq!(rig.fake.started(), 2);
    assert!(rig.fake.start_of(1).prompt.contains("Try the other provider."));
    assert_ne!(rig.fake.start_of(0).key, rig.fake.start_of(1).key, "a second run has its own key");
    assert_eq!(rig.service.task(&key).await.unwrap().task.task.runs.len(), 2);
}

#[tokio::test]
async fn a_run_that_cannot_start_is_recorded_and_waits_for_a_person() {
    let rig = rig();
    let ops = project(&rig, "OPS").await;
    let key = task(&rig, &ops, "Rotate the key", Some(scout())).await;
    rig.fake.0.lock().start_error = Some("There is no agent called scout.".into());
    let moved = rig.service.move_task(&key, to(TaskState::InProgress), None).await.unwrap();
    assert_eq!(moved.state, TaskState::InProgress);
    let detail = rig.service.task(&key).await.unwrap();
    assert_eq!(detail.task.needs_you, Some(NeedsYou::RunFailed));
    assert!(
        detail
            .events
            .iter()
            .any(|e| matches!(&e.kind, ProjectEventKind::StartFailed { reason } if reason.contains("no agent")))
    );
    // Fixing the cause and running again works; the beat does not retry on its own.
    rig.fake.0.lock().start_error = None;
    assert_eq!(rig.service.reconcile().await.unwrap(), 0);
    assert_eq!(rig.fake.started(), 0);
    rig.service.restart_task(&key, None, None).await.unwrap();
    assert_eq!(rig.fake.started(), 1);
}

#[tokio::test]
async fn a_task_waits_for_what_it_depends_on_and_starts_when_that_is_done() {
    let rig = rig();
    let ops = project(&rig, "OPS").await;
    let first = task(&rig, &ops, "Alert on slow queries", Some(Who::You)).await;
    let second = rig
        .service
        .add_task(
            &ops,
            NewTask {
                title: "Migrate alerts".into(),
                assignee: Some(scout()),
                depends_on: vec![first.clone()],
                ..NewTask::default()
            },
            None,
        )
        .await
        .unwrap()
        .key;
    rig.service.move_task(&second, to(TaskState::InProgress), None).await.unwrap();
    assert_eq!(rig.fake.started(), 0, "it is blocked");
    let detail = rig.service.task(&second).await.unwrap();
    assert_eq!(detail.task.blocked_by, vec![first.clone()]);
    assert!(
        detail.events.iter().any(|e| matches!(&e.kind, ProjectEventKind::Blocked { by } if by == &vec![first.clone()]))
    );
    assert_eq!(rig.service.reconcile().await.unwrap(), 0);

    rig.service.move_task(&first, to(TaskState::Done), None).await.unwrap();
    assert_eq!(rig.service.reconcile().await.unwrap(), 1);
    assert_eq!(rig.fake.started(), 1);
    assert!(
        rig.fake
            .start_of(0)
            .prompt
            .contains("OPS-1: <data source=\"task.title\" untrusted=\"true\">Alert on slow queries</data>"),
        "it is told what it builds on"
    );
}

#[tokio::test]
async fn a_cycle_or_a_missing_dependency_is_refused() {
    let rig = rig();
    let ops = project(&rig, "OPS").await;
    let a = task(&rig, &ops, "A", None).await;
    let b = rig
        .service
        .add_task(&ops, NewTask { title: "B".into(), depends_on: vec![a.clone()], ..NewTask::default() }, None)
        .await
        .unwrap()
        .key;
    let mut fields = TaskFields::of(&rig.service.task(&a).await.unwrap().task.task);
    fields.depends_on = vec![b.clone()];
    let error = rig.service.update_task(&a, fields.clone(), None).await.unwrap_err();
    assert!(error.to_string().contains("wait for itself"), "{error}");
    fields.depends_on = vec!["OPS-99".parse().unwrap()];
    assert!(rig.service.update_task(&a, fields, None).await.unwrap_err().to_string().contains("does not exist"));
}

#[tokio::test]
async fn moving_a_task_out_of_a_run_that_is_going_says_so_first_and_then_stops_it() {
    let rig = rig();
    let ops = project(&rig, "OPS").await;
    let key = task(&rig, &ops, "Triage", Some(scout())).await;
    rig.service.move_task(&key, to(TaskState::InProgress), None).await.unwrap();
    let error = rig.service.move_task(&key, to(TaskState::Backlog), None).await.unwrap_err();
    assert!(error.to_string().contains(RUN_GOING), "{error}");
    assert!(rig.fake.0.lock().cancelled.is_empty(), "nothing was stopped without a yes");
    let back = rig
        .service
        .move_task(&key, Move { to: Some(TaskState::Backlog), cancel_run: true, ..Move::default() }, None)
        .await
        .unwrap();
    assert_eq!(back.state, TaskState::Backlog);
    assert_eq!(rig.fake.0.lock().cancelled.len(), 1);
    let detail = rig.service.task(&key).await.unwrap();
    assert_eq!(detail.task.task.runs[0].outcome, RunOutcome::Cancelled);
    assert!(detail.task.task.run_requested.is_none());
}

#[tokio::test]
async fn sending_a_task_back_with_a_note_runs_it_again_with_the_note() {
    let rig = rig();
    let ops = project(&rig, "OPS").await;
    let key = task(&rig, &ops, "Runbook", Some(scout())).await;
    rig.service.move_task(&key, to(TaskState::InProgress), None).await.unwrap();
    rig.fake.finish(0, "Draft 1");
    assert_eq!(rig.service.task(&key).await.unwrap().task.task.state, TaskState::Review);
    rig.service
        .move_task(
            &key,
            Move { to: Some(TaskState::InProgress), note: Some("Add the Friday renewal.".into()), ..Move::default() },
            Some(Who::You),
        )
        .await
        .unwrap();
    assert_eq!(rig.fake.started(), 2);
    assert!(rig.fake.start_of(1).prompt.contains("Add the Friday renewal."));
    let t = rig.service.task(&key).await.unwrap().task.task;
    assert_eq!(t.runs.len(), 2);
    assert_eq!(t.runs[1].note.as_deref(), Some("Add the Friday renewal."));
    assert_eq!(t.deliverables.len(), 1, "the first answer stays");
}

#[tokio::test]
async fn a_card_is_placed_where_it_is_dropped_even_when_the_lane_runs_out_of_room() {
    let rig = rig();
    let ops = project(&rig, "OPS").await;
    let first = task(&rig, &ops, "One", None).await;
    let last = task(&rig, &ops, "Two", None).await;
    let mut movers = Vec::new();
    for n in 0..12 {
        movers.push(task(&rig, &ops, &format!("Mover {n}"), None).await);
    }
    // Each card goes to place 1, between the first card and the one before it: the gap halves until the lane is numbered again.
    for key in &movers {
        rig.service
            .move_task(key, Move { to: Some(TaskState::Backlog), index: Some(1), ..Move::default() }, None)
            .await
            .unwrap();
    }
    let board = rig.service.board(&ops).await.unwrap();
    let mut lane: Vec<&zorix_core::Task> =
        board.tasks.iter().map(|t| &t.task).filter(|t| t.state == TaskState::Backlog).collect();
    lane.sort_by_key(|t| t.rank);
    let order: Vec<String> = lane.iter().map(|t| t.key.to_string()).collect();
    assert_eq!(order.first().map(String::as_str), Some(first.to_string().as_str()));
    let mut expected: Vec<String> = movers.iter().rev().map(ToString::to_string).collect();
    expected.insert(0, first.to_string());
    expected.push(last.to_string());
    assert_eq!(order, expected, "the newest drop is right after the first card");
    let ranks: Vec<i64> = lane.iter().map(|t| t.rank).collect();
    assert!(ranks.windows(2).all(|w| w[0] < w[1]), "ranks stay strictly ordered: {ranks:?}");
}

#[tokio::test]
async fn asking_the_manager_needs_one_and_gives_it_only_its_project() {
    let rig = rig();
    let ops = project(&rig, "OPS").await;
    let id = rig.service.ask_manager(&ops, Some("What is late?")).await.unwrap();
    assert_eq!(rig.fake.started(), 1);
    let start = rig.fake.start_of(0);
    assert_eq!(start.target, Target::Agent { agent: atlas(), version: None });
    assert_eq!(start.access, vec!["project:OPS:write".to_string()]);
    assert!(start.prompt.contains("What is late?"));
    let board = rig.service.board(&ops).await.unwrap();
    assert_eq!(board.manager_run.as_ref().map(|m| m.execution), Some(id));
    rig.fake.finish(0, "Done planning");
    assert!(rig.service.board(&ops).await.unwrap().manager_run.is_none(), "it is over");

    let web = ProjectKey::new("WEB").unwrap();
    rig.service
        .create_project(
            NewProject {
                key: web.clone(),
                name: "Web".into(),
                description: String::new(),
                manager: None,
                start: None,
                due: None,
                cost_limit_micros: None,
            },
            None,
        )
        .await
        .unwrap();
    assert!(rig.service.ask_manager(&web, None).await.unwrap_err().to_string().contains("no manager"));
}

fn binding(slug: &str) -> AgentBinding {
    AgentBinding {
        agent: AgentId::new(),
        slug: AgentSlug::new(slug).unwrap(),
        version: 1,
        hash: "h".into(),
        skills: Vec::new(),
        allowed_models: vec!["*".into()],
        agent_memory: false,
        team: None,
        autonomy: None,
    }
}

fn context(entries: &[&str], slug: &str) -> ToolContext {
    let entries: Vec<String> = entries.iter().map(ToString::to_string).collect();
    ToolContext {
        execution_id: ExecutionId::new(),
        tenant: TenantId::local(),
        call_id: ToolCallId("c".into()),
        grants: access::compile_access(&entries).unwrap(),
        cancel: CancellationToken::new(),
        workspace_root: None,
        conversation: None,
        agent: Some(binding(slug)),
        host: None,
    }
}

fn tool(rig: &Rig, id: &str) -> Arc<dyn Tool> {
    project_tools(Arc::downgrade(&rig.service)).into_iter().find(|t| t.spec().id == id).unwrap()
}

#[tokio::test]
async fn the_managers_writes_ask_and_a_move_needs_a_person_who_is_looking() {
    let rig = rig();
    for id in ["tasks.create", "tasks.update", "tasks.move", "tasks.comment"] {
        assert!(tool(&rig, id).spec().requires_approval, "{id} asks");
    }
    assert!(tool(&rig, "tasks.move").spec().approval_by_person_only);
    assert!(!tool(&rig, "tasks.create").spec().approval_by_person_only);
    for id in ["tasks.list", "tasks.get"] {
        assert!(!tool(&rig, id).spec().requires_approval && tool(&rig, id).spec().annotations.read_only, "{id} reads");
    }
    for id in ["task.report_progress", "task.add_deliverable", "task.note"] {
        assert!(!tool(&rig, id).spec().requires_approval, "{id} changes only its own task");
    }
    // The cards the person sees say what would change.
    assert_eq!(
        tool(&rig, "tasks.move").describe_call(&json!({"task": "OPS-38", "to": "done"})).as_deref(),
        Some("Move OPS-38 to done")
    );
}

#[tokio::test]
async fn the_manager_reads_and_plans_only_inside_its_project() {
    let rig = rig();
    let ops = project(&rig, "OPS").await;
    let web = project(&rig, "WEB").await;
    let manage = context(&["project:OPS:write"], "atlas");
    let created = tool(&rig, "tasks.create")
        .call(&manage, json!({"project": "OPS", "title": "Draft the on-call schedule", "assignee": "agent:scout", "due": "2026-12-04"}))
        .await
        .unwrap();
    assert_eq!(created["created"], "OPS-1");
    let listed = tool(&rig, "tasks.list").call(&manage, json!({"project": "OPS"})).await.unwrap();
    assert_eq!(listed["tasks"][0]["key"], "OPS-1");
    assert_eq!(listed["tasks"][0]["assignee"], "scout");
    // Another project is out of reach, to read and to write.
    let other =
        tool(&rig, "tasks.create").call(&manage, json!({"project": "WEB", "title": "Sneaky"})).await.unwrap_err();
    assert!(other.to_string().contains("no permission"), "{other}");
    assert!(tool(&rig, "tasks.list").call(&manage, json!({"project": "WEB"})).await.is_err());
    // A read-only grant lists but cannot plan.
    let reader = context(&["project:OPS"], "atlas");
    assert!(tool(&rig, "tasks.list").call(&reader, json!({"project": "OPS"})).await.is_ok());
    assert!(tool(&rig, "tasks.comment").call(&reader, json!({"task": "OPS-1", "text": "hi"})).await.is_err());
    // A move records who did it and that a person approved it.
    tool(&rig, "tasks.move").call(&manage, json!({"task": "OPS-1", "to": "done"})).await.unwrap();
    let history = rig.service.history(&ops, None, 20).await.unwrap();
    let moved = history.iter().find(|e| matches!(e.kind, ProjectEventKind::Moved { to: TaskState::Done, .. })).unwrap();
    assert_eq!(moved.by, Some(Who::Agent { slug: atlas() }));
    assert!(matches!(&moved.kind, ProjectEventKind::Moved { approved_by: Some(a), .. } if a == "you"));
    assert!(rig.service.board(&web).await.unwrap().tasks.is_empty());
}

#[tokio::test]
async fn a_task_runs_own_tools_reach_only_that_task() {
    let rig = rig();
    let ops = project(&rig, "OPS").await;
    let one = task(&rig, &ops, "Triage", Some(scout())).await;
    let two = task(&rig, &ops, "Other", Some(scout())).await;
    let mine = context(&["task:OPS-1"], "scout");
    tool(&rig, "task.report_progress").call(&mine, json!({"task": "OPS-1", "percent": 64})).await.unwrap();
    tool(&rig, "task.add_deliverable")
        .call(&mine, json!({"task": "OPS-1", "kind": "answer", "title": "Sorted list", "body": "12 items"}))
        .await
        .unwrap();
    tool(&rig, "task.add_deliverable")
        .call(
            &mine,
            json!({"task": "OPS-1", "kind": "link", "title": "Tracker view", "body": "https://example.com/view"}),
        )
        .await
        .unwrap();
    tool(&rig, "task.note").call(&mine, json!({"task": "OPS-1", "text": "Three look like duplicates."})).await.unwrap();
    let t = rig.service.task(&one).await.unwrap().task.task;
    assert_eq!((t.progress, t.deliverables.len()), (64, 2));
    assert_eq!(t.deliverables[0].execution, Some(mine.execution_id));
    // Not its neighbour, and not the planning tools.
    assert!(tool(&rig, "task.report_progress").call(&mine, json!({"task": "OPS-2", "percent": 10})).await.is_err());
    assert_eq!(rig.service.task(&two).await.unwrap().task.task.progress, 0);
    assert!(!mine.grants.allows(&Permission::use_tool("tasks.move")));
    assert!(!mine.grants.allows(&Permission::new(Action::Write, "project:OPS")));
}

#[tokio::test]
async fn projects_are_listed_with_their_figures_and_archiving_hides_them() {
    let rig = rig();
    let ops = project(&rig, "OPS").await;
    let a = task(&rig, &ops, "A", Some(scout())).await;
    let b = task(&rig, &ops, "B", Some(Who::You)).await;
    let _c = task(&rig, &ops, "C", None).await;
    rig.service.move_task(&a, to(TaskState::InProgress), None).await.unwrap();
    rig.fake.finish(0, "Done");
    rig.service.move_task(&b, to(TaskState::Done), None).await.unwrap();
    let cards = rig.service.projects(false).await.unwrap();
    assert_eq!(cards.len(), 1);
    let card = &cards[0];
    assert_eq!((card.counts.backlog, card.counts.review, card.counts.done, card.counts.total()), (1, 1, 1, 3));
    assert_eq!(card.needs_you, 1, "the task in Review");
    assert_eq!(card.assignees.len(), 2);
    let mut fields = zorix_projects::ProjectFields::of(&card.project);
    fields.archived = true;
    rig.service.update_project(&ops, fields, None).await.unwrap();
    assert!(rig.service.projects(false).await.unwrap().is_empty());
    assert_eq!(rig.service.projects(true).await.unwrap().len(), 1);
    let error =
        rig.service.add_task(&ops, NewTask { title: "x".into(), ..NewTask::default() }, None).await.unwrap_err();
    assert!(error.to_string().contains("archived"));
}

#[tokio::test]
async fn what_needs_a_person_and_the_index_of_tasks_span_the_active_projects() {
    let rig = rig();
    let ops = project(&rig, "OPS").await;
    let web = project(&rig, "WEB").await;
    let a = task(&rig, &ops, "Triage", Some(scout())).await;
    let _b = task(&rig, &ops, "Backlog item", None).await;
    let c = task(&rig, &web, "Landing page", Some(Who::You)).await;
    rig.service.move_task(&a, to(TaskState::InProgress), None).await.unwrap();
    rig.fake.finish(0, "Done");
    rig.service.reconcile().await.unwrap();
    rig.service.move_task(&c, to(TaskState::Review), None).await.unwrap();
    let needs = rig.service.needing_you().await.unwrap();
    let keys: Vec<String> = needs.iter().map(|n| n.key.to_string()).collect();
    assert_eq!(keys.len(), 2);
    assert!(keys.contains(&"OPS-1".to_string()) && keys.contains(&"WEB-1".to_string()));
    assert!(needs.iter().all(|n| n.needs == NeedsYou::Review));
    assert_eq!(
        needs.iter().find(|n| n.project.as_str() == "WEB").map(|n| n.project_name.as_str()),
        Some("Project WEB")
    );
    assert_eq!(rig.service.index().await.unwrap().len(), 3);
    // An archived project leaves both.
    let mut fields = zorix_projects::ProjectFields::of(&rig.service.project(&web).await.unwrap());
    fields.archived = true;
    rig.service.update_project(&web, fields, None).await.unwrap();
    assert_eq!(rig.service.needing_you().await.unwrap().len(), 1);
    assert_eq!(rig.service.index().await.unwrap().len(), 2);
}
