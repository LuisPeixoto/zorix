//! The workforce end to end with a scripted launcher, stored executions and a manual clock: hiring and the rules of the organisation,
//! figures that count each cost once, a worker's level and budgets shaping the runs it starts, a manager's plan and report, the
//! schedule that is off until turned on, and `org.team` showing a manager only its own reports.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zorix_core::{
    AgentBinding, AgentSlug, AgentSpec, AgentStore, AutonomyLevel, Date, Execution, ExecutionId, ExecutionRepository,
    ExecutionState, ManualClock, NewExecution, OrgEventKind, OrgStore, ProjectKey, ReportKind, ReportStatus,
    TaskRequest, TenantId, Timestamp, ToolCallId, ToolContext, Who, access,
};
use zorix_projects::{NewProject, NewTask, ProjectService, RunShaper};
use zorix_runtime::MemoryRepository;
use zorix_store::{SqliteAgentStore, SqliteOrgStore, SqliteProjectStore};
use zorix_workflows::launcher::{Outcome, RunStart, StepLauncher, Target, ToolCall};
use zorix_workforce::{NewWorker, SettingsFields, WorkerFields, WorkforceService, org_tools};

/// Wednesday 2026-11-18 12:00 UTC. The week of the tests starts on Monday 2026-11-16 00:00 UTC.
const WEDNESDAY: i64 = 1_795_003_200_000;
const MONDAY: i64 = 1_794_787_200_000;
const DAY: i64 = 86_400_000;

#[derive(Default)]
struct World {
    started: Vec<(ExecutionId, RunStart)>,
    outcomes: HashMap<ExecutionId, Outcome>,
}

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<World>>);

impl Fake {
    fn count(&self) -> usize {
        self.0.lock().started.len()
    }

    fn start_of(&self, index: usize) -> RunStart {
        self.0.lock().started[index].1.clone()
    }

    fn finish(&self, index: usize, text: &str) {
        let mut world = self.0.lock();
        let id = world.started[index].0;
        world.outcomes.insert(id, Outcome::Completed { text: text.into(), structured: None, cost_micros: 1, steps: 2 });
    }
}

#[async_trait]
impl StepLauncher for Fake {
    async fn start(&self, start: RunStart) -> Result<ExecutionId, String> {
        let mut world = self.0.lock();
        if let Some((id, _)) = world.started.iter().find(|(_, s)| s.key == start.key) {
            return Ok(*id);
        }
        let id = ExecutionId::new();
        world.outcomes.insert(id, Outcome::Running { cost_micros: 0 });
        world.started.push((id, start));
        Ok(id)
    }

    async fn outcome(&self, id: ExecutionId) -> Result<Outcome, String> {
        self.0.lock().outcomes.get(&id).cloned().ok_or_else(|| "unknown".into())
    }

    async fn cancel(&self, _id: ExecutionId, _reason: &str) -> Result<(), String> {
        Ok(())
    }

    async fn call_tool(&self, _call: &ToolCall) -> Result<Value, String> {
        Ok(json!({}))
    }
}

struct Rig {
    service: Arc<WorkforceService>,
    projects: Arc<ProjectService>,
    fake: Fake,
    clock: Arc<ManualClock>,
    repo: Arc<MemoryRepository>,
    agents: Arc<SqliteAgentStore>,
    org: Arc<SqliteOrgStore>,
}

fn slug(s: &str) -> AgentSlug {
    AgentSlug::new(s).unwrap()
}

async fn rig(names: &[&str]) -> Rig {
    let tenant = TenantId::local();
    let org = Arc::new(SqliteOrgStore::open_in_memory().unwrap());
    let project_store = Arc::new(SqliteProjectStore::open_in_memory().unwrap());
    let agents = Arc::new(SqliteAgentStore::open_in_memory().unwrap());
    for name in names {
        let spec = AgentSpec::new(slug(name), name.to_string(), "demo");
        agents.create_agent(&tenant, spec, None, Timestamp(1)).await.unwrap();
    }
    let repo = Arc::new(MemoryRepository::new());
    let fake = Fake::default();
    let clock = Arc::new(ManualClock::new(Timestamp(WEDNESDAY)));
    let service = Arc::new(
        WorkforceService::new(
            org.clone(),
            project_store.clone(),
            agents.clone(),
            repo.clone(),
            Arc::new(fake.clone()),
            tenant.clone(),
            clock.clone(),
        )
        .in_zone("UTC"),
    );
    let projects = Arc::new(ProjectService::new(project_store, Arc::new(fake.clone()), tenant, clock.clone()));
    projects.set_shaper(Some(service.clone()));
    Rig { service, projects, fake, clock, repo, agents, org }
}

fn new_worker(agent: &str, dept: &str, boss: Option<&str>, level: AutonomyLevel) -> NewWorker {
    NewWorker {
        agent: slug(agent),
        role: format!("{agent} role"),
        department: dept.into(),
        reports_to: boss.map(slug),
        manager: false,
        autonomy: Some(level),
        weekly_budget_micros: None,
        task_time_ms: None,
        still_ask: vec![],
    }
}

async fn binding(rig: &Rig, name: &str) -> AgentBinding {
    let agent = rig.agents.agent_by_slug(&TenantId::local(), &slug(name)).await.unwrap().unwrap();
    let version = rig.agents.version(&TenantId::local(), agent.id, agent.current_version).await.unwrap().unwrap();
    AgentBinding::of(&agent, &version)
}

/// Store an execution of an agent with a cost, as if it had run.
async fn stored_run(
    rig: &Rig,
    name: &str,
    parent: Option<ExecutionId>,
    cost: u64,
    at: i64,
    state: ExecutionState,
    eval: bool,
) -> ExecutionId {
    let mut metadata = std::collections::BTreeMap::new();
    if eval {
        metadata.insert(zorix_core::EVAL_LABEL.to_string(), "1".to_string());
    }
    let new = NewExecution {
        request: TaskRequest { prompt: "work".into(), model: "demo".into(), metadata, ..TaskRequest::default() },
        parent,
        agent: Some(binding(rig, name).await),
        ..NewExecution::default()
    };
    let mut execution = Execution::new(ExecutionId::new(), new, Timestamp(at));
    execution.usage.cost_micros = cost;
    execution.state = state;
    let id = execution.id;
    rig.repo.create(execution, vec![]).await.unwrap();
    id
}

fn usd(n: u64) -> u64 {
    n * 1_000_000
}

#[tokio::test]
async fn hiring_makes_an_agent_a_worker_at_the_organisations_default_level_and_refuses_what_does_not_fit() {
    let rig = rig(&["atlas", "forge"]).await;
    let atlas = rig
        .service
        .hire(
            NewWorker { autonomy: None, manager: true, ..new_worker("atlas", "operations", None, AutonomyLevel::Ask) },
            "you",
        )
        .await
        .unwrap();
    assert_eq!(atlas.autonomy, AutonomyLevel::Ask, "the default level for new workers");
    assert!(atlas.manager && !atlas.schedule.enabled, "the schedule is off");
    assert_eq!(atlas.task_time_ms, 2 * 60 * 60 * 1_000);

    let twice = rig.service.hire(new_worker("atlas", "operations", None, AutonomyLevel::Ask), "you").await;
    assert!(twice.unwrap_err().to_string().contains("already a worker"));
    let ghost = rig.service.hire(new_worker("ghost", "operations", None, AutonomyLevel::Ask), "you").await;
    assert!(ghost.unwrap_err().to_string().contains("no agent called ghost"));
    let nowhere = rig.service.hire(new_worker("forge", "sales", None, AutonomyLevel::Ask), "you").await;
    assert!(nowhere.unwrap_err().to_string().contains("does not exist"));
    // Freer than the manager is refused.
    let freer = rig.service.hire(new_worker("forge", "engineering", Some("atlas"), AutonomyLevel::Full), "you").await;
    assert!(freer.unwrap_err().to_string().contains("freer than atlas"));
    rig.service.hire(new_worker("forge", "engineering", Some("atlas"), AutonomyLevel::Ask), "you").await.unwrap();
    let history = rig.org.events(&TenantId::local(), None, 10).await.unwrap_or_default();
    assert_eq!(history.len(), 2);
    assert!(history.iter().all(|e| e.kind == OrgEventKind::Hired));
}

#[tokio::test]
async fn a_manager_cannot_be_lowered_below_its_reports_and_cannot_be_let_go_while_it_has_them() {
    let rig = rig(&["atlas", "forge"]).await;
    rig.service.hire(new_worker("atlas", "operations", None, AutonomyLevel::Budgeted), "you").await.unwrap();
    rig.service.hire(new_worker("forge", "engineering", Some("atlas"), AutonomyLevel::Notify), "you").await.unwrap();

    let atlas = rig.service.worker(&slug("atlas")).await.unwrap();
    let mut fields = WorkerFields::of(&atlas);
    fields.autonomy = AutonomyLevel::Ask;
    let lowered = rig.service.edit(&slug("atlas"), fields, "you").await;
    assert!(lowered.unwrap_err().to_string().contains("freer than atlas"), "lower the reports first");

    let fire = rig.service.fire(&slug("atlas"), "you").await;
    assert!(fire.unwrap_err().to_string().contains("has reports (forge)"));

    // Lowering the report first, then the manager, works, and the level change is recorded.
    let forge = rig.service.worker(&slug("forge")).await.unwrap();
    let mut f = WorkerFields::of(&forge);
    f.autonomy = AutonomyLevel::Ask;
    rig.service.edit(&slug("forge"), f, "you").await.unwrap();
    let mut a = WorkerFields::of(&atlas);
    a.autonomy = AutonomyLevel::Ask;
    rig.service.edit(&slug("atlas"), a, "you").await.unwrap();
    let kinds: Vec<OrgEventKind> =
        rig.org.events(&TenantId::local(), None, 10).await.unwrap().into_iter().map(|e| e.kind).collect();
    assert_eq!(kinds.iter().filter(|k| **k == OrgEventKind::LevelChanged).count(), 2);

    rig.service.fire(&slug("forge"), "you").await.unwrap();
    rig.service.fire(&slug("atlas"), "you").await.unwrap();
    assert!(rig.service.workers().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_department_with_workers_cannot_be_removed_and_the_settings_keep_what_was_read() {
    let rig = rig(&["atlas"]).await;
    rig.service.hire(new_worker("atlas", "operations", None, AutonomyLevel::Ask), "you").await.unwrap();
    let mut settings = rig.service.settings().await.unwrap();
    let fields = SettingsFields {
        weekly_budget_micros: Some(usd(120)),
        default_level: AutonomyLevel::Notify,
        departments: settings.departments.iter().filter(|d| d.slug != "operations").cloned().collect(),
    };
    let refused = rig.service.save_settings(fields.clone(), "you").await;
    assert!(refused.unwrap_err().to_string().contains("department “operations”, which does not exist"));
    rig.service.mark_seen().await.unwrap();
    settings = rig.service.settings().await.unwrap();
    let seen = settings.seen_until;
    assert!(seen.is_some());
    let ok = SettingsFields { departments: settings.departments.clone(), ..fields };
    let saved = rig.service.save_settings(ok, "you").await.unwrap();
    assert_eq!((saved.weekly_budget_micros, saved.default_level), (Some(usd(120)), AutonomyLevel::Notify));
    assert_eq!(saved.seen_until, seen, "saving the settings does not mark notices unread");
}

#[tokio::test]
async fn spend_counts_each_cost_once_leaves_out_evaluations_and_old_weeks_and_a_manager_includes_its_tree() {
    let rig = rig(&["atlas", "forge", "scout"]).await;
    rig.service.hire(new_worker("atlas", "operations", None, AutonomyLevel::Ask), "you").await.unwrap();
    rig.service.hire(new_worker("forge", "engineering", Some("atlas"), AutonomyLevel::Ask), "you").await.unwrap();
    // Atlas started a run that cost $10 in all; Forge did $4 of it as a child, so Atlas's own work cost $6.
    let root = stored_run(&rig, "atlas", None, usd(10), WEDNESDAY - DAY, ExecutionState::Completed, false).await;
    stored_run(&rig, "forge", Some(root), usd(4), WEDNESDAY - DAY + 1, ExecutionState::Completed, false).await;
    // An evaluation and last week's run do not count.
    stored_run(&rig, "forge", None, usd(50), WEDNESDAY - DAY, ExecutionState::Completed, true).await;
    stored_run(&rig, "forge", None, usd(30), MONDAY - DAY, ExecutionState::Completed, false).await;

    let overview = rig.service.overview().await.unwrap();
    let by = |name: &str| overview.workers.iter().find(|w| w.worker.agent.as_str() == name).unwrap();
    assert_eq!(by("atlas").own.week_micros, usd(6));
    assert_eq!(by("forge").own.week_micros, usd(4));
    assert_eq!(by("atlas").figures.week_micros, usd(10), "a manager's figures include everyone below it");
    assert_eq!(by("forge").figures.week_micros, usd(4));
    assert_eq!(overview.week_micros, usd(10), "the organisation counts root runs: the child is inside it");
    assert_eq!(overview.week, "2026-11-16");
}

#[tokio::test]
async fn figures_count_open_tasks_and_the_share_done_on_time_and_what_waits_for_a_person() {
    let rig = rig(&["forge"]).await;
    rig.service.hire(new_worker("forge", "engineering", None, AutonomyLevel::Ask), "you").await.unwrap();
    let ops = ProjectKey::new("OPS").unwrap();
    rig.projects
        .create_project(
            NewProject {
                key: ops.clone(),
                name: "Ops".into(),
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
    let who = Some(Who::Agent { slug: slug("forge") });
    let make = |title: &str, due: Option<&str>| NewTask {
        title: title.into(),
        assignee: who.clone(),
        due: due.map(|d| Date::new(d).unwrap()),
        ..NewTask::default()
    };
    let a = rig.projects.add_task(&ops, make("late one", Some("2026-11-17")), None).await.unwrap().key;
    let b = rig.projects.add_task(&ops, make("early one", Some("2026-11-20")), None).await.unwrap().key;
    rig.projects.add_task(&ops, make("still open", Some("2026-11-19")), None).await.unwrap();
    // Done on the 18th: the first was due on the 17th (late), the second on the 20th (on time).
    for key in [&a, &b] {
        rig.projects
            .move_task(key, zorix_projects::Move { to: Some(zorix_core::TaskState::Done), ..Default::default() }, None)
            .await
            .unwrap();
    }
    stored_run(&rig, "forge", None, 0, WEDNESDAY - 10, ExecutionState::WaitingForApproval, false).await;

    let detail = rig.service.detail(&slug("forge")).await.unwrap();
    assert_eq!(detail.view.own.tasks, 1);
    assert_eq!((detail.view.own.on_time_done, detail.view.own.on_time_all), (1, 2));
    assert_eq!(detail.view.own.waiting, 1);
    assert_eq!(detail.view.status, Some("warn"));
    assert_eq!(detail.tasks.len(), 1);
    assert_eq!(detail.tasks[0].title, "still open");
    let overview = rig.service.overview().await.unwrap();
    assert_eq!((overview.open_tasks, overview.due_this_week, overview.waiting_approvals), (1, 1, 1));
}

fn start_for(agent: &str) -> RunStart {
    RunStart {
        target: Target::Agent { agent: slug(agent), version: None },
        prompt: "do it".into(),
        input: None,
        model: None,
        max_cost_micros: Some(usd(5)),
        key: format!("k-{agent}"),
        metadata: Default::default(),
        access: vec![],
        max_active_ms: None,
        autonomy: None,
    }
}

#[tokio::test]
async fn a_workers_run_starts_at_its_level_with_its_time_and_what_is_left_of_its_budget() {
    let rig = rig(&["forge", "stranger"]).await;
    let mut hire = new_worker("forge", "engineering", None, AutonomyLevel::Budgeted);
    hire.weekly_budget_micros = Some(usd(25));
    hire.still_ask = vec!["repo:acme/web:write".into()];
    hire.task_time_ms = Some(60 * 60 * 1_000);
    rig.service.hire(hire, "you").await.unwrap();
    stored_run(&rig, "forge", None, usd(22), WEDNESDAY - 5, ExecutionState::Completed, false).await;

    let shaped = rig.service.shape(start_for("forge")).await.unwrap();
    let autonomy = shaped.autonomy.clone().unwrap();
    assert_eq!(autonomy.level, AutonomyLevel::Budgeted);
    assert_eq!(autonomy.still_ask, vec!["repo:acme/web:write".to_string()]);
    assert_eq!(shaped.max_active_ms, Some(60 * 60 * 1_000));
    assert_eq!(shaped.max_cost_micros, Some(usd(3)), "$3 is left of the $25 week, less than the $5 asked");
    assert_eq!(shaped.metadata.get("worker").map(String::as_str), Some("forge"));

    // Once the week is spent a new run starts at Ask, and says so.
    stored_run(&rig, "forge", None, usd(3), WEDNESDAY - 4, ExecutionState::Completed, false).await;
    let spent = rig.service.shape(start_for("forge")).await.unwrap();
    assert_eq!(spent.autonomy.unwrap().level, AutonomyLevel::Ask);
    assert!(spent.metadata.get("autonomy_note").unwrap().contains("weekly budget is spent"));

    // An agent that is not a worker is left alone.
    let untouched = rig.service.shape(start_for("stranger")).await.unwrap();
    assert_eq!(untouched, start_for("stranger"));
}

#[tokio::test]
async fn a_task_run_under_the_workforce_is_shaped_by_its_worker() {
    let rig = rig(&["forge"]).await;
    rig.service.hire(new_worker("forge", "engineering", None, AutonomyLevel::Notify), "you").await.unwrap();
    let ops = ProjectKey::new("OPS").unwrap();
    rig.projects
        .create_project(
            NewProject {
                key: ops.clone(),
                name: "Ops".into(),
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
    let task = rig
        .projects
        .add_task(
            &ops,
            NewTask {
                title: "Fix the check".into(),
                assignee: Some(Who::Agent { slug: slug("forge") }),
                ..NewTask::default()
            },
            None,
        )
        .await
        .unwrap();
    rig.projects
        .move_task(
            &task.key,
            zorix_projects::Move { to: Some(zorix_core::TaskState::InProgress), ..Default::default() },
            None,
        )
        .await
        .unwrap();
    assert_eq!(rig.fake.count(), 1);
    let start = rig.fake.start_of(0);
    assert_eq!(start.autonomy.unwrap().level, AutonomyLevel::Notify);
    assert_eq!(start.max_active_ms, Some(2 * 60 * 60 * 1_000));
}

async fn manager(rig: &Rig) {
    let mut hire = new_worker("atlas", "operations", None, AutonomyLevel::Budgeted);
    hire.manager = true;
    rig.service.hire(hire, "you").await.unwrap();
}

#[tokio::test]
async fn planning_the_week_starts_the_managers_run_with_its_team_and_projects_and_keeps_the_answer() {
    let rig = rig(&["atlas"]).await;
    manager(&rig).await;
    let ops = ProjectKey::new("OPS").unwrap();
    rig.projects
        .create_project(
            NewProject {
                key: ops.clone(),
                name: "Ops".into(),
                description: String::new(),
                manager: Some(slug("atlas")),
                start: None,
                due: None,
                cost_limit_micros: None,
            },
            None,
        )
        .await
        .unwrap();

    let report = rig.service.plan_week(&slug("atlas"), Some("Focus on the migration.")).await.unwrap();
    assert_eq!(
        (report.kind, report.status, report.week.as_str()),
        (ReportKind::Plan, ReportStatus::Running, "2026-11-16")
    );
    let start = rig.fake.start_of(0);
    assert!(
        start.prompt.contains("2026-11-16")
            && start.prompt.contains("OPS")
            && start.prompt.contains("Focus on the migration.")
    );
    assert_eq!(start.access, vec!["org:team".to_string(), "project:OPS:write".to_string()]);
    assert_eq!(start.autonomy.unwrap().level, AutonomyLevel::Budgeted);

    // Nothing is kept until the run is over.
    rig.service.reconcile().await.unwrap();
    assert_eq!(rig.service.reports(Some(&slug("atlas")), 10).await.unwrap()[0].status, ReportStatus::Running);
    rig.fake.finish(0, "Nine tasks assigned.");
    rig.service.reconcile().await.unwrap();
    let kept = rig.service.reports(Some(&slug("atlas")), 10).await.unwrap();
    assert_eq!((kept[0].status, kept[0].body.as_str()), (ReportStatus::Done, "Nine tasks assigned."));
    let notices = rig.service.notices().await.unwrap();
    assert!(notices.iter().any(|n| n.kind == "report" && n.title.contains("planned the week")));
    rig.service.mark_seen().await.unwrap();
    assert!(rig.service.notices().await.unwrap().is_empty(), "read notices are gone");
}

#[tokio::test]
async fn only_a_manager_plans_and_a_report_asks_for_read_access_to_the_projects() {
    let rig = rig(&["atlas", "forge"]).await;
    manager(&rig).await;
    rig.service.hire(new_worker("forge", "engineering", Some("atlas"), AutonomyLevel::Ask), "you").await.unwrap();
    let not_manager = rig.service.plan_week(&slug("forge"), None).await;
    assert!(not_manager.unwrap_err().to_string().contains("not a manager"));
    let ops = ProjectKey::new("OPS").unwrap();
    rig.projects
        .create_project(
            NewProject {
                key: ops,
                name: "Ops".into(),
                description: String::new(),
                manager: Some(slug("atlas")),
                start: None,
                due: None,
                cost_limit_micros: None,
            },
            None,
        )
        .await
        .unwrap();
    rig.service.write_report(&slug("atlas"), None).await.unwrap();
    let start = rig.fake.start_of(0);
    assert_eq!(
        start.access,
        vec!["org:team".to_string(), "project:OPS".to_string()],
        "a report reads and changes nothing"
    );
    assert!(start.prompt.contains("Do not change any task"));
}

#[tokio::test]
async fn the_schedule_is_off_until_turned_on_and_a_slot_runs_once_and_a_missed_one_is_only_recorded() {
    let rig = rig(&["atlas"]).await;
    manager(&rig).await;
    // Off: even after Monday 09:00 nothing runs.
    rig.clock.advance((MONDAY + 7 * DAY + 9 * 3_600_000 + 30_000 - WEDNESDAY) as u64);
    rig.service.reconcile().await.unwrap();
    assert_eq!(rig.fake.count(), 0);

    // Turn it on at Monday 09:00:30: the slot that just went by is not run.
    let atlas = rig.service.worker(&slug("atlas")).await.unwrap();
    let mut fields = WorkerFields::of(&atlas);
    fields.schedule_enabled = true;
    rig.service.edit(&slug("atlas"), fields, "you").await.unwrap();
    rig.service.reconcile().await.unwrap();
    assert_eq!(rig.fake.count(), 0, "turning it on does not catch up");

    // The next Monday at 09:00:30 the plan slot fires once, and the Friday report is not due.
    rig.clock.advance((7 * DAY) as u64);
    rig.service.reconcile().await.unwrap();
    assert_eq!(rig.fake.count(), 1);
    assert!(rig.fake.start_of(0).prompt.contains("Plan the week"));
    rig.service.reconcile().await.unwrap();
    assert_eq!(rig.fake.count(), 1, "the same slot is not run twice");

    // Friday 16:00 starts the report. A whole week of closed laptop only records what was missed.
    rig.clock.advance((4 * DAY + 7 * 3_600_000) as u64);
    rig.service.reconcile().await.unwrap();
    assert_eq!(rig.fake.count(), 2);
    rig.clock.advance((14 * DAY + 3 * 3_600_000) as u64);
    rig.service.reconcile().await.unwrap();
    assert_eq!(rig.fake.count(), 2, "slots from a long absence are not run");
    let events = rig.org.events(&TenantId::local(), None, 50).await.unwrap();
    assert!(events.iter().any(|e| e.kind == OrgEventKind::ScheduleMissed), "the miss is recorded");
}

fn context(entries: &[&str], agent: Option<AgentBinding>) -> ToolContext {
    ToolContext {
        execution_id: ExecutionId::new(),
        tenant: TenantId::local(),
        call_id: ToolCallId("c".into()),
        grants: access::compile_access(&entries.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap(),
        cancel: CancellationToken::new(),
        workspace_root: None,
        conversation: None,
        agent,
        host: None,
    }
}

#[tokio::test]
async fn org_team_shows_a_manager_only_the_workers_below_it_and_needs_its_grant() {
    let rig = rig(&["atlas", "forge", "scout"]).await;
    manager(&rig).await;
    rig.service.hire(new_worker("forge", "engineering", Some("atlas"), AutonomyLevel::Ask), "you").await.unwrap();
    rig.service.hire(new_worker("scout", "research", None, AutonomyLevel::Ask), "you").await.unwrap();
    let tool = org_tools(Arc::downgrade(&rig.service)).remove(0);
    let atlas = binding(&rig, "atlas").await;

    let denied = tool.call(&context(&[], Some(atlas.clone())), json!({})).await;
    assert!(denied.unwrap_err().to_string().contains("org:team"));
    let seen = tool.call(&context(&["org:team"], Some(atlas)), json!({})).await.unwrap();
    let names: Vec<&str> = seen["team"].as_array().unwrap().iter().map(|w| w["agent"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["forge"], "scout answers to you, not to atlas");
    assert_eq!(seen["manager"], "atlas");
    let nobody = tool.call(&context(&["org:team"], None), json!({})).await;
    assert!(nobody.unwrap_err().to_string().contains("Only an agent has a team"));
}

#[tokio::test]
async fn budget_notices_are_recorded_once_a_week_when_a_worker_nears_and_spends_its_budget() {
    let rig = rig(&["forge"]).await;
    let mut hire = new_worker("forge", "engineering", None, AutonomyLevel::Ask);
    hire.weekly_budget_micros = Some(usd(25));
    rig.service.hire(hire, "you").await.unwrap();
    stored_run(&rig, "forge", None, usd(21), WEDNESDAY - 100, ExecutionState::Completed, false).await;
    rig.service.reconcile().await.unwrap();
    rig.service.reconcile().await.unwrap();
    let near: Vec<_> = rig
        .org
        .events(&TenantId::local(), None, 50)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == OrgEventKind::BudgetNear)
        .collect();
    assert_eq!(near.len(), 1, "once, not on every beat");
    assert!(near[0].text.contains("84%"));

    stored_run(&rig, "forge", None, usd(5), WEDNESDAY - 50, ExecutionState::Completed, false).await;
    rig.service.reconcile().await.unwrap();
    rig.service.reconcile().await.unwrap();
    let kinds: Vec<OrgEventKind> =
        rig.org.events(&TenantId::local(), None, 50).await.unwrap().into_iter().map(|e| e.kind).collect();
    assert_eq!(kinds.iter().filter(|k| **k == OrgEventKind::BudgetSpent).count(), 1);
    let lines = rig.service.activity(20).await.unwrap();
    assert!(lines.iter().any(|l| l.level == "WARN" && l.text.contains("spent its weekly budget")));
}

#[tokio::test]
async fn calls_that_ran_without_asking_and_proposals_become_notices_and_a_failed_run_is_an_error_line() {
    use zorix_core::{Autonomy, EventKind, NewEvent};
    let rig = rig(&["forge", "writer"]).await;
    rig.service.hire(new_worker("forge", "engineering", None, AutonomyLevel::Notify), "you").await.unwrap();
    rig.service.hire(new_worker("writer", "operations", None, AutonomyLevel::Suggest), "you").await.unwrap();

    let run = |name: &'static str, level: AutonomyLevel, state: ExecutionState, events: Vec<NewEvent>| {
        let rig = &rig;
        async move {
            let mut bound = binding(rig, name).await;
            bound.autonomy = Some(Autonomy::new(level));
            let new = NewExecution {
                request: TaskRequest { prompt: "work".into(), model: "demo".into(), ..TaskRequest::default() },
                agent: Some(bound),
                ..NewExecution::default()
            };
            let mut execution = Execution::new(ExecutionId::new(), new, Timestamp(WEDNESDAY - 60_000));
            execution.state = state;
            rig.repo.create(execution, events).await.unwrap();
        }
    };
    let at = Timestamp(WEDNESDAY - 30_000);
    run(
        "forge",
        AutonomyLevel::Notify,
        ExecutionState::Completed,
        vec![NewEvent::new(
            EventKind::ToolUnattended,
            at,
            json!({"tool": "github.create_pull_request", "summary": "Open a pull request on acme/web", "level": "notify"}),
        )],
    )
    .await;
    run(
        "writer",
        AutonomyLevel::Suggest,
        ExecutionState::Completed,
        vec![NewEvent::new(
            EventKind::ToolSuggested,
            at,
            json!({"tool": "slack.send_message", "summary": "Send the Q4 summary to #finance", "arguments": {}}),
        )],
    )
    .await;
    run("forge", AutonomyLevel::Notify, ExecutionState::Failed, vec![]).await;

    let notices = rig.service.notices().await.unwrap();
    let kinds: Vec<&str> = notices.iter().map(|n| n.kind).collect();
    assert!(kinds.contains(&"notice") && kinds.contains(&"suggestion"), "{kinds:?}");
    let suggestion = notices.iter().find(|n| n.kind == "suggestion").unwrap();
    assert!(
        suggestion.text.contains("Send the Q4 summary to #finance") && suggestion.worker.as_deref() == Some("writer")
    );
    let lines = rig.service.activity(20).await.unwrap();
    assert!(lines.iter().any(|l| l.kind == "unattended" && l.text.contains("Open a pull request")));
    assert!(lines.iter().any(|l| l.level == "ERR" && l.kind == "failed"));
}
