//! Contract tests for the workflow store: versions are immutable and not duplicated, unattended actions need a confirmation that
//! is recorded and carried only while the step is unchanged, runs are written with a revision, a wait is settled once, and
//! tenants never mix.

use serde_json::json;
use zorix_core::{
    AgentSlug, Ask, Confirm, Edge, Fired, RunState, Service, Step, StepKind, StepRun, TenantId, Then, Timestamp,
    TriggerState, WaitDecision, WaitId, WaitKind, WaitStatus, WorkflowId, WorkflowRun, WorkflowRunId, WorkflowSlug,
    WorkflowSpec, WorkflowStore, WorkflowWait,
};
use zorix_store::SqliteWorkflowStore;

fn tenant(name: &str) -> TenantId {
    TenantId::new(name).unwrap()
}

const NOW: Timestamp = Timestamp(1_000);

fn slug(name: &str) -> WorkflowSlug {
    WorkflowSlug::new(name).unwrap()
}

fn spec() -> WorkflowSpec {
    let mut spec = WorkflowSpec::blank("Daily digest");
    spec.steps.push(Step {
        id: "ask".into(),
        title: "Ask the researcher".into(),
        x: 0,
        y: 0,
        kind: StepKind::Agent {
            agent: AgentSlug::new("researcher").unwrap(),
            prompt: "What changed?".into(),
            version: None,
            model: None,
            max_cost_micros: None,
        },
    });
    spec.edges.push(Edge { from: "start".into(), to: "ask".into(), label: None });
    spec
}

fn with_message(mut spec: WorkflowSpec, ask: Ask, text: &str) -> WorkflowSpec {
    spec.steps.retain(|s| s.id != "post");
    spec.edges.retain(|e| e.to != "post");
    spec.steps.push(Step {
        id: "post".into(),
        title: "Post to #ops".into(),
        x: 0,
        y: 0,
        kind: StepKind::Send {
            service: Service::Slack,
            connection: None,
            channel: "#ops".into(),
            text: text.into(),
            ask,
        },
    });
    spec.edges.push(Edge { from: "ask".into(), to: "post".into(), label: None });
    spec
}

fn run_of(workflow: &zorix_core::Workflow, state: RunState, at: i64) -> WorkflowRun {
    WorkflowRun {
        id: WorkflowRunId::new(),
        tenant: workflow.tenant.clone(),
        workflow: workflow.id,
        slug: workflow.slug.clone(),
        version: Some(1),
        test: false,
        spec: workflow.draft.clone(),
        fired: Fired { step: "start".into(), label: "Manual".into(), payload: json!({}), at: Timestamp(at) },
        state,
        steps: [("start".to_string(), StepRun::default())].into(),
        cost_micros: 0,
        error: None,
        chain: 0,
        from_run: None,
        started_at: Timestamp(at),
        finished_at: None,
        revision: 0,
    }
}

fn wait_of(run: &WorkflowRun, at: i64, due: Option<i64>) -> WorkflowWait {
    WorkflowWait {
        id: WaitId::new(),
        tenant: run.tenant.clone(),
        run: run.id,
        workflow: run.workflow,
        step: "approve".into(),
        kind: WaitKind::Approval,
        title: "On-call approves".into(),
        message: "Go ahead?".into(),
        shown: json!({"severity": 4}),
        fingerprint: "abc".into(),
        status: WaitStatus::Pending,
        due_at: due.map(Timestamp),
        then: Then::Refuse,
        default_answer: None,
        remind_at: None,
        reminded: false,
        created_at: Timestamp(at),
        decided_at: None,
        decided_by: None,
        answer: None,
        note: None,
    }
}

#[tokio::test]
async fn a_workflow_starts_as_a_draft_that_is_off_and_a_slug_is_taken_once_per_tenant() {
    let store = SqliteWorkflowStore::open_in_memory().unwrap();
    let a = tenant("a");
    let workflow = store.create_workflow(&a, slug("daily"), spec(), NOW).await.unwrap();
    assert!(!workflow.enabled);
    assert_eq!(workflow.live_version, None);
    assert!(store.versions(&a, workflow.id).await.unwrap().is_empty());
    assert!(store.create_workflow(&a, slug("daily"), spec(), NOW).await.is_err());
    store.create_workflow(&tenant("b"), slug("daily"), spec(), NOW).await.unwrap();
    assert_eq!(store.workflows(&a).await.unwrap().len(), 1);
    assert!(store.workflow(&tenant("b"), workflow.id).await.unwrap().is_none());
    assert!(store.workflow_by_slug(&a, &slug("daily")).await.unwrap().is_some());
}

#[tokio::test]
async fn an_invalid_draft_is_refused() {
    let store = SqliteWorkflowStore::open_in_memory().unwrap();
    let a = tenant("a");
    let mut bad = spec();
    bad.edges.clear();
    assert!(store.create_workflow(&a, slug("daily"), bad.clone(), NOW).await.is_err());
    let workflow = store.create_workflow(&a, slug("daily"), spec(), NOW).await.unwrap();
    assert!(store.save_draft(&a, workflow.id, bad, NOW).await.is_err());
}

#[tokio::test]
async fn publishing_writes_the_next_version_and_never_the_same_one_twice() {
    let store = SqliteWorkflowStore::open_in_memory().unwrap();
    let a = tenant("a");
    let workflow = store.create_workflow(&a, slug("daily"), spec(), NOW).await.unwrap();
    let (v1, written) = store.publish(&a, workflow.id, Some("first".into()), None, NOW).await.unwrap();
    assert!(written);
    assert_eq!(v1.version, 1);
    let (again, written) = store.publish(&a, workflow.id, None, None, NOW).await.unwrap();
    assert!(!written);
    assert_eq!(again.version, 1);

    // Moving a node is not a change.
    let mut moved = spec();
    moved.steps[1].x = 500;
    store.save_draft(&a, workflow.id, moved, NOW).await.unwrap();
    assert!(!store.publish(&a, workflow.id, None, None, NOW).await.unwrap().1);

    let mut changed = spec();
    changed.steps[1].title = "Ask again".into();
    store.save_draft(&a, workflow.id, changed, Timestamp(2_000)).await.unwrap();
    let (v2, written) = store.publish(&a, workflow.id, Some("second".into()), None, Timestamp(2_000)).await.unwrap();
    assert!(written);
    assert_eq!(v2.version, 2);
    let versions = store.versions(&a, workflow.id).await.unwrap();
    assert_eq!(versions.iter().map(|v| v.version).collect::<Vec<_>>(), vec![2, 1]);
    assert_eq!(store.version(&a, workflow.id, 1).await.unwrap().unwrap().spec.steps[1].title, "Ask the researcher");
    assert_eq!(store.workflow(&a, workflow.id).await.unwrap().unwrap().live_version, Some(2));
}

#[tokio::test]
async fn an_unattended_action_needs_a_confirmation_that_is_recorded_and_dropped_when_the_step_changes() {
    let store = SqliteWorkflowStore::open_in_memory().unwrap();
    let a = tenant("a");
    let workflow = store.create_workflow(&a, slug("daily"), spec(), NOW).await.unwrap();
    store.save_draft(&a, workflow.id, with_message(spec(), Ask::Never, "hello"), NOW).await.unwrap();

    let error = store.publish(&a, workflow.id, None, None, NOW).await.unwrap_err().to_string();
    assert!(error.contains("confirm") && error.contains("post"), "{error}");

    let me = Confirm { by: "luis".into() };
    let (v1, _) = store.publish(&a, workflow.id, None, Some(me.clone()), NOW).await.unwrap();
    assert_eq!(v1.confirmations.len(), 1);
    assert_eq!((v1.confirmations[0].step.as_str(), v1.confirmations[0].by.as_str()), ("post", "luis"));

    // Another change that leaves the action alone keeps its confirmation without asking again.
    let mut other = with_message(spec(), Ask::Never, "hello");
    other.steps[1].title = "Ask the researcher, carefully".into();
    store.save_draft(&a, workflow.id, other, Timestamp(2_000)).await.unwrap();
    let (v2, _) = store.publish(&a, workflow.id, None, None, Timestamp(2_000)).await.unwrap();
    assert_eq!(v2.confirmations, v1.confirmations);

    // Editing the action drops the confirmation.
    let mut edited = with_message(spec(), Ask::Never, "a different message");
    edited.steps[1].title = "Ask the researcher, carefully".into();
    store.save_draft(&a, workflow.id, edited, Timestamp(3_000)).await.unwrap();
    assert!(store.publish(&a, workflow.id, None, None, Timestamp(3_000)).await.is_err());
    let (v3, _) = store.publish(&a, workflow.id, None, Some(me), Timestamp(3_000)).await.unwrap();
    assert_eq!(v3.confirmations[0].at, Timestamp(3_000));

    // Asking every time needs no confirmation.
    store.save_draft(&a, workflow.id, with_message(spec(), Ask::Always, "hello"), Timestamp(4_000)).await.unwrap();
    assert!(store.publish(&a, workflow.id, None, None, Timestamp(4_000)).await.unwrap().1);
}

#[tokio::test]
async fn rollback_writes_a_new_version_and_resets_the_draft() {
    let store = SqliteWorkflowStore::open_in_memory().unwrap();
    let a = tenant("a");
    let workflow = store.create_workflow(&a, slug("daily"), spec(), NOW).await.unwrap();
    store.publish(&a, workflow.id, None, None, NOW).await.unwrap();
    let mut changed = spec();
    changed.steps[1].title = "Ask again".into();
    store.save_draft(&a, workflow.id, changed, NOW).await.unwrap();
    store.publish(&a, workflow.id, None, None, NOW).await.unwrap();

    let back = store.rollback(&a, workflow.id, 1, None, Timestamp(2_000)).await.unwrap();
    assert_eq!(back.version, 3);
    assert_eq!(back.note.as_deref(), Some("rollback to v1"));
    assert_eq!(back.spec.steps[1].title, "Ask the researcher");
    let now = store.workflow(&a, workflow.id).await.unwrap().unwrap();
    assert_eq!(now.draft.steps[1].title, "Ask the researcher");
    assert_eq!(now.live_version, Some(3));
    assert!(store.rollback(&a, workflow.id, 9, None, NOW).await.is_err());
}

#[tokio::test]
async fn only_a_published_workflow_can_be_turned_on_and_removing_it_keeps_its_runs() {
    let store = SqliteWorkflowStore::open_in_memory().unwrap();
    let a = tenant("a");
    let workflow = store.create_workflow(&a, slug("daily"), spec(), NOW).await.unwrap();
    assert!(store.set_enabled(&a, workflow.id, true, NOW).await.is_err());
    store.publish(&a, workflow.id, None, None, NOW).await.unwrap();
    store.set_enabled(&a, workflow.id, true, NOW).await.unwrap();
    assert!(store.workflow(&a, workflow.id).await.unwrap().unwrap().enabled);
    store.set_enabled(&a, workflow.id, false, NOW).await.unwrap();

    let run = run_of(&workflow, RunState::Completed, 5);
    assert!(store.put_run(&a, &run, None).await.unwrap());
    assert!(!store.remove_workflow(&tenant("b"), workflow.id).await.unwrap());
    assert!(store.remove_workflow(&a, workflow.id).await.unwrap());
    assert!(store.workflow(&a, workflow.id).await.unwrap().is_none());
    assert!(store.run(&a, run.id).await.unwrap().is_some());
}

#[tokio::test]
async fn a_run_is_written_with_a_revision_so_two_writers_cannot_overwrite_each_other() {
    let store = SqliteWorkflowStore::open_in_memory().unwrap();
    let a = tenant("a");
    let workflow = store.create_workflow(&a, slug("daily"), spec(), NOW).await.unwrap();
    let mut run = run_of(&workflow, RunState::Running, 5);
    assert!(store.put_run(&a, &run, None).await.unwrap());
    assert!(!store.put_run(&a, &run, None).await.unwrap(), "a second insert of the same run is refused");

    let loaded = store.run(&a, run.id).await.unwrap().unwrap();
    assert_eq!(loaded.revision, 0);
    run.state = RunState::Waiting;
    assert!(store.put_run(&a, &run, Some(0)).await.unwrap());
    assert!(!store.put_run(&a, &run, Some(0)).await.unwrap(), "revision 0 is stale now");
    let loaded = store.run(&a, run.id).await.unwrap().unwrap();
    assert_eq!((loaded.revision, loaded.state), (1, RunState::Waiting));
    assert!(store.run(&tenant("b"), run.id).await.unwrap().is_none());
}

#[tokio::test]
async fn runs_list_newest_first_and_active_ones_are_the_unfinished() {
    let store = SqliteWorkflowStore::open_in_memory().unwrap();
    let a = tenant("a");
    let one = store.create_workflow(&a, slug("one"), spec(), NOW).await.unwrap();
    let two = store.create_workflow(&a, slug("two"), spec(), NOW).await.unwrap();
    let old = run_of(&one, RunState::Completed, 10);
    let waiting = run_of(&one, RunState::Waiting, 20);
    let other = run_of(&two, RunState::Running, 30);
    for run in [&old, &waiting, &other] {
        store.put_run(&a, run, None).await.unwrap();
    }
    let of_one = store.runs(&a, Some(one.id), 10).await.unwrap();
    assert_eq!(of_one.iter().map(|r| r.id).collect::<Vec<_>>(), vec![waiting.id, old.id]);
    assert_eq!(
        store.runs(&a, None, 2).await.unwrap().iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![other.id, waiting.id]
    );
    let active: Vec<_> = store.active_runs(&a).await.unwrap().into_iter().map(|r| r.id).collect();
    assert_eq!(active, vec![waiting.id, other.id]);
    assert!(store.runs(&tenant("b"), None, 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_wait_is_settled_once_and_pending_ones_come_oldest_first() {
    let store = SqliteWorkflowStore::open_in_memory().unwrap();
    let a = tenant("a");
    let workflow = store.create_workflow(&a, slug("daily"), spec(), NOW).await.unwrap();
    let run = run_of(&workflow, RunState::Waiting, 5);
    store.put_run(&a, &run, None).await.unwrap();
    let late = wait_of(&run, 200, Some(9_000));
    let early = wait_of(&run, 100, None);
    store.put_wait(&a, &late).await.unwrap();
    store.put_wait(&a, &early).await.unwrap();
    let pending = store.pending_waits(&a).await.unwrap();
    assert_eq!(pending.iter().map(|w| w.id).collect::<Vec<_>>(), vec![early.id, late.id]);
    assert!(store.pending_waits(&tenant("b")).await.unwrap().is_empty());

    let done = store
        .settle_wait(
            &a,
            early.id,
            WaitDecision {
                status: WaitStatus::Approved,
                by: Some("luis".into()),
                answer: None,
                note: Some("fine".into()),
            },
            Timestamp(300),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (done.status, done.decided_by.as_deref(), done.decided_at),
        (WaitStatus::Approved, Some("luis"), Some(Timestamp(300)))
    );
    let second = store.settle_wait(&a, early.id, WaitDecision::of(WaitStatus::Refused), Timestamp(400)).await.unwrap();
    assert!(second.is_none(), "an answered wait cannot be answered again");
    assert_eq!(store.wait(&a, early.id).await.unwrap().unwrap().status, WaitStatus::Approved);
    assert_eq!(store.pending_waits(&a).await.unwrap().len(), 1);
    assert_eq!(store.waits_of(&a, run.id).await.unwrap().len(), 2);

    store.mark_reminded(&a, late.id).await.unwrap();
    assert!(store.wait(&a, late.id).await.unwrap().unwrap().reminded);
}

#[tokio::test]
async fn trigger_state_and_meta_are_kept_and_cleared() {
    let store = SqliteWorkflowStore::open_in_memory().unwrap();
    let a = tenant("a");
    let id = WorkflowId::new();
    let state = TriggerState {
        workflow: id,
        step: "start".into(),
        next_at: Some(Timestamp(9)),
        last_fired: None,
        missed: 2,
        last_missed: Some(Timestamp(3)),
    };
    store.put_trigger_state(&a, &state).await.unwrap();
    let mut newer = state.clone();
    newer.missed = 3;
    store.put_trigger_state(&a, &newer).await.unwrap();
    assert_eq!(store.trigger_states(&a).await.unwrap(), vec![newer]);
    assert!(store.trigger_states(&tenant("b")).await.unwrap().is_empty());
    store.clear_trigger_states(&a, id).await.unwrap();
    assert!(store.trigger_states(&a).await.unwrap().is_empty());

    assert_eq!(store.meta(&a, "cursor").await.unwrap(), None);
    store.set_meta(&a, "cursor", "12").await.unwrap();
    store.set_meta(&a, "cursor", "13").await.unwrap();
    assert_eq!(store.meta(&a, "cursor").await.unwrap().as_deref(), Some("13"));
}
