//! The service over a real hub on a throwaway data folder: the list and its figures, files that round-trip, what a workflow names
//! that is missing, and the history of one step.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use zorix_core::{
    AgentSlug, AgentSpec, AgentStore, Ask, Clock, Confirm, Edge, ExecutionId, ManualClock, RunState, Service, Step,
    StepKind, TenantId, Then, Timestamp, WaitStatus, WorkflowSlug, WorkflowSpec, WorkflowStore,
};
use zorix_hub::Hub;
use zorix_providers::MemoryStore;
use zorix_runtime::InMemoryToolRegistry;
use zorix_store::{SqliteWorkflowStore, database_path};
use zorix_workflows::{
    Decision, Health, Outcome, RunStart, StartRequest, StepLauncher, ToolCall, Verdict, WorkflowEngine, WorkflowService,
};

struct Instant;

#[async_trait]
impl StepLauncher for Instant {
    async fn start(&self, _start: RunStart) -> Result<ExecutionId, String> {
        Ok(ExecutionId::new())
    }
    async fn outcome(&self, _id: ExecutionId) -> Result<Outcome, String> {
        Ok(Outcome::Completed { text: "done".into(), structured: None, cost_micros: 0, steps: 1 })
    }
    async fn cancel(&self, _id: ExecutionId, _reason: &str) -> Result<(), String> {
        Ok(())
    }
    async fn call_tool(&self, _call: &ToolCall) -> Result<Value, String> {
        Ok(json!({}))
    }
}

struct Rig {
    service: WorkflowService,
    engine: Arc<WorkflowEngine>,
    store: Arc<SqliteWorkflowStore>,
    hub: Arc<Hub>,
    clock: Arc<ManualClock>,
    tenant: TenantId,
    _home: tempfile::TempDir,
}

fn rig() -> Rig {
    let home = tempfile::tempdir().unwrap();
    let tenant = TenantId::new("local").unwrap();
    let secrets = Arc::new(MemoryStore::default());
    let registry = Arc::new(InMemoryToolRegistry::new());
    let hub = Arc::new(Hub::open(home.path(), tenant.clone(), secrets, registry).unwrap());
    let store = Arc::new(SqliteWorkflowStore::open(&database_path(home.path())).unwrap());
    let clock = Arc::new(ManualClock::new(Timestamp(1_795_161_570_000)));
    let engine = Arc::new(WorkflowEngine::new(store.clone(), Arc::new(Instant), clock.clone(), tenant.clone()));
    let service = WorkflowService::new(store.clone(), tenant.clone(), hub.clone(), clock.clone());
    Rig { service, engine, store, hub, clock, tenant, _home: home }
}

fn step(id: &str, kind: StepKind) -> Step {
    Step { id: id.into(), title: id.into(), x: 0, y: 0, kind }
}

fn simple(name: &str) -> WorkflowSpec {
    let mut spec = WorkflowSpec::blank(name);
    spec.steps.push(step(
        "a",
        StepKind::Agent {
            agent: AgentSlug::new("researcher").unwrap(),
            prompt: "go".into(),
            version: None,
            model: None,
            max_cost_micros: None,
        },
    ));
    spec.edges.push(Edge { from: "start".into(), to: "a".into(), label: None });
    spec
}

async fn publish(rig: &Rig, slug: &str, spec: WorkflowSpec) -> zorix_core::Workflow {
    let workflow = rig.service.create(WorkflowSlug::new(slug).unwrap(), spec).await.unwrap();
    rig.service.publish(workflow.id, None, Some(Confirm { by: "t".into() })).await.unwrap();
    rig.service.set_enabled(workflow.id, true).await.unwrap();
    rig.store.workflow(&rig.tenant, workflow.id).await.unwrap().unwrap()
}

async fn run_once(rig: &Rig, id: zorix_core::WorkflowId) -> zorix_core::WorkflowRun {
    let run = rig.engine.start(StartRequest::manual(id, json!({}))).await.unwrap();
    rig.engine.advance(run.id).await.unwrap()
}

#[tokio::test]
async fn the_list_shows_what_starts_each_workflow_its_last_runs_and_how_it_is_doing() {
    let rig = rig();
    let healthy = publish(&rig, "healthy", simple("Healthy")).await;
    let paused = publish(&rig, "paused", simple("Paused")).await;
    rig.service.set_enabled(paused.id, false).await.unwrap();
    let mut waiting_spec = simple("Waiting");
    waiting_spec.steps.push(step(
        "ok",
        StepKind::Approval { message: "Go?".into(), show: vec![], timeout_ms: None, then: Then::Refuse, remind: None },
    ));
    waiting_spec.edges.push(Edge { from: "a".into(), to: "ok".into(), label: None });
    let waiting = publish(&rig, "waiting", waiting_spec).await;

    for _ in 0..16 {
        run_once(&rig, healthy.id).await;
    }
    run_once(&rig, waiting.id).await;

    let cards = rig.service.cards().await.unwrap();
    let by = |slug: &str| cards.iter().find(|c| c.workflow.slug.as_str() == slug).unwrap();
    assert_eq!(by("healthy").bars.len(), 14, "only the last 14 runs are bars");
    assert_eq!(by("healthy").health, Health::Healthy);
    assert_eq!(by("healthy").triggers[0].label, "Manual");
    assert_eq!(by("paused").health, Health::Paused);
    assert_eq!(by("waiting").health, Health::Waiting { count: 1 });
    assert_eq!(by("waiting").bars.last().unwrap().state, RunState::Waiting);
    assert!(!by("healthy").unpublished_changes);

    // An edit that is not published yet is flagged.
    let mut edited = simple("Healthy");
    edited.description = "Now with a description".into();
    rig.service.save_draft(healthy.id, edited).await.unwrap();
    assert!(
        rig.service
            .cards()
            .await
            .unwrap()
            .iter()
            .find(|c| c.workflow.slug.as_str() == "healthy")
            .unwrap()
            .unpublished_changes
    );
}

#[tokio::test]
async fn the_figures_say_nothing_they_do_not_know() {
    let rig = rig();
    let empty = rig.service.figures().await.unwrap();
    assert_eq!((empty.active, empty.paused, empty.runs_week, empty.waiting), (0, 0, 0, 0));
    assert_eq!(empty.finished_percent, None);
    assert_eq!(empty.saved_minutes, None);

    let flow = publish(&rig, "flow", simple("Flow")).await;
    run_once(&rig, flow.id).await;
    let figures = rig.service.figures().await.unwrap();
    assert_eq!((figures.active, figures.runs_week), (1, 1));
    assert_eq!(figures.finished_percent, Some(100.0));
    assert_eq!(figures.saved_minutes, None, "no workflow says how much a run saves, so no number is invented");

    let mut saving = simple("Flow");
    saving.saves_minutes = Some(20);
    rig.service.save_draft(flow.id, saving).await.unwrap();
    rig.service.publish(flow.id, None, None).await.unwrap();
    run_once(&rig, flow.id).await;
    run_once(&rig, flow.id).await;
    assert_eq!(rig.service.figures().await.unwrap().saved_minutes, Some(40));
}

#[tokio::test]
async fn the_oldest_wait_is_how_long_a_person_has_been_waited_for() {
    let rig = rig();
    let mut spec = simple("Ask");
    spec.steps.push(step(
        "ok",
        StepKind::Approval { message: "Go?".into(), show: vec![], timeout_ms: None, then: Then::Refuse, remind: None },
    ));
    spec.edges.push(Edge { from: "a".into(), to: "ok".into(), label: None });
    let flow = publish(&rig, "ask", spec).await;
    run_once(&rig, flow.id).await;
    rig.clock.advance(14 * 60_000);
    let figures = rig.service.figures().await.unwrap();
    assert_eq!(figures.waiting, 1);
    assert_eq!(figures.oldest_wait_ms, Some(14 * 60_000));
}

#[tokio::test]
async fn files_round_trip_and_an_import_only_touches_the_draft() {
    let rig = rig();
    let flow = publish(&rig, "flow", simple("Flow")).await;
    let text = rig.service.export(flow.id, None).await.unwrap();
    assert!(text.contains("\"zorix\": \"workflow/1\"") && text.ends_with("}\n"));

    let again = rig.service.import(&text).await.unwrap();
    assert!(!again.created && !again.changed);

    let edited = text.replace("\"prompt\": \"go\"", "\"prompt\": \"go further\"");
    let imported = rig.service.import(&edited).await.unwrap();
    assert!(!imported.created && imported.changed);
    assert_eq!(
        rig.store.workflow(&rig.tenant, flow.id).await.unwrap().unwrap().live_version,
        Some(1),
        "nothing was published"
    );
    assert_eq!(
        rig.service.export(flow.id, Some(1)).await.unwrap(),
        text.replace("\"slug\": \"flow\",", "\"slug\": \"flow\",\n  \"version\": 1,")
    );

    let brand_new = text.replace("\"slug\": \"flow\"", "\"slug\": \"copy\"");
    let made = rig.service.import(&brand_new).await.unwrap();
    assert!(made.created);
    assert!(
        !made.workflow.enabled && made.workflow.live_version.is_none(),
        "an imported workflow is a draft that is off"
    );
    assert!(rig.service.import("{}").await.is_err());
}

#[tokio::test]
async fn a_starter_is_added_as_a_draft_that_is_off_and_only_once() {
    let rig = rig();
    let made = rig.service.add_template("incident-triage").await.unwrap();
    assert!(!made.enabled && made.live_version.is_none());
    assert_eq!(made.draft.name, "Incident triage");
    assert!(rig.service.add_template("incident-triage").await.is_err(), "the name is taken");
    assert!(rig.service.add_template("nope").await.unwrap_err().to_string().contains("no starter"));
}

#[tokio::test]
async fn what_a_workflow_names_and_the_computer_lacks_is_listed() {
    let rig = rig();
    let made = rig.service.add_template("weekly-report").await.unwrap();
    let missing = rig.service.missing(&made.draft).await.unwrap();
    let names: Vec<(&str, &str)> = missing.iter().map(|m| (m.kind, m.name.as_str())).collect();
    assert_eq!(names, vec![("team", "research-desk"), ("connection", "slack")]);

    // An agent that exists is no longer missing.
    rig.hub
        .agents
        .create_agent(
            &rig.tenant,
            AgentSpec::new(AgentSlug::new("researcher").unwrap(), "Researcher", "demo"),
            None,
            Timestamp(1),
        )
        .await
        .unwrap();
    let digest = rig.service.add_template("daily-digest").await.unwrap();
    let names: Vec<&str> = rig.service.missing(&digest.draft).await.unwrap().iter().map(|m| m.kind).collect();
    assert_eq!(names, vec!["connection"], "only the Slack connection is still missing");
}

#[tokio::test]
async fn publishing_with_a_message_that_runs_without_asking_names_the_steps_to_confirm() {
    let rig = rig();
    let mut spec = simple("Notify");
    spec.steps.push(step(
        "post",
        StepKind::Send {
            service: Service::Slack,
            connection: None,
            channel: "#ops".into(),
            text: "hi".into(),
            ask: Ask::Never,
        },
    ));
    spec.edges.push(Edge { from: "a".into(), to: "post".into(), label: None });
    let flow = rig.service.create(WorkflowSlug::new("notify").unwrap(), spec).await.unwrap();
    assert_eq!(rig.service.to_confirm(flow.id).await.unwrap(), vec!["post"]);
    assert!(rig.service.publish(flow.id, None, None).await.unwrap_err().to_string().contains("confirm"));
    rig.service.publish(flow.id, None, Some(Confirm { by: "luis".into() })).await.unwrap();
    assert!(rig.service.to_confirm(flow.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn the_history_of_a_step_gives_its_last_outcomes_and_how_long_people_took() {
    let rig = rig();
    let mut spec = simple("Ask");
    spec.steps.push(step(
        "ok",
        StepKind::Approval { message: "Go?".into(), show: vec![], timeout_ms: None, then: Then::Refuse, remind: None },
    ));
    spec.edges.push(Edge { from: "a".into(), to: "ok".into(), label: None });
    let flow = publish(&rig, "ask", spec).await;
    for minutes in [5u64, 11, 20] {
        let run = run_once(&rig, flow.id).await;
        rig.clock.advance(minutes * 60_000);
        let wait = rig.service.waits_of(run.id).await.unwrap().remove(0);
        let decision =
            Decision { fingerprint: wait.fingerprint, verdict: Verdict::Approve, by: "luis".into(), note: None };
        rig.engine.decide(wait.id, decision).await.unwrap();
        assert_eq!(rig.store.wait(&rig.tenant, wait.id).await.unwrap().unwrap().status, WaitStatus::Approved);
    }
    let history = rig.service.step_history(flow.id, "ok", 5).await.unwrap();
    assert_eq!(history.outcomes.len(), 3);
    assert_eq!(history.median_wait_ms, Some(11 * 60_000));
    assert_eq!(rig.service.step_history(flow.id, "nope", 5).await.unwrap().outcomes.len(), 0);
    let _ = rig.clock.now();
}
