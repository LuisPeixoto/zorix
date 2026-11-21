//! The engine end to end with a scripted launcher and a manual clock: agent, condition, approval and action in order; a join that
//! waits for both branches; the branch not taken; budgets that narrow; a test run that asks before every action; time-outs;
//! a restart in the middle of a wait; and a person's answer that must name what they were shown.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::{Value, json};
use zorix_core::{
    AgentSlug, Ask, Edge, ExecutionId, ManualClock, Remind, RunState, Service, Step, StepKind, StepState, TenantId,
    Then, Timestamp, WaitKind, WaitStatus, WorkflowId, WorkflowSlug, WorkflowSpec, WorkflowStore,
};
use zorix_store::SqliteWorkflowStore;
use zorix_workflows::{Decision, Outcome, RunStart, StartRequest, StepLauncher, ToolCall, Verdict, WorkflowEngine};

#[derive(Default)]
struct World {
    started: Vec<(ExecutionId, RunStart)>,
    outcomes: HashMap<ExecutionId, Outcome>,
    calls: Vec<ToolCall>,
    cancelled: Vec<ExecutionId>,
    call_error: Option<String>,
}

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<World>>);

impl Fake {
    fn finish(&self, index: usize, text: &str, cost: u64) {
        let mut world = self.0.lock();
        let id = world.started[index].0;
        world
            .outcomes
            .insert(id, Outcome::Completed { text: text.into(), structured: None, cost_micros: cost, steps: 3 });
    }

    fn fail(&self, index: usize, reason: &str) {
        let mut world = self.0.lock();
        let id = world.started[index].0;
        world.outcomes.insert(id, Outcome::Failed { reason: reason.into(), cost_micros: 0 });
    }

    fn wait_for_person(&self, index: usize) {
        let mut world = self.0.lock();
        let id = world.started[index].0;
        world.outcomes.insert(id, Outcome::WaitingForPerson { cost_micros: 10 });
    }

    fn started(&self) -> usize {
        self.0.lock().started.len()
    }

    fn start_of(&self, index: usize) -> RunStart {
        self.0.lock().started[index].1.clone()
    }

    fn calls(&self) -> Vec<ToolCall> {
        self.0.lock().calls.clone()
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
        self.0.lock().outcomes.get(&id).cloned().ok_or_else(|| "unknown execution".into())
    }

    async fn cancel(&self, id: ExecutionId, _reason: &str) -> Result<(), String> {
        let mut world = self.0.lock();
        world.cancelled.push(id);
        world.outcomes.insert(id, Outcome::Cancelled);
        Ok(())
    }

    async fn call_tool(&self, call: &ToolCall) -> Result<Value, String> {
        let mut world = self.0.lock();
        world.calls.push(call.clone());
        match &world.call_error {
            Some(error) => Err(error.clone()),
            None => Ok(json!({"ok": true})),
        }
    }
}

struct Rig {
    engine: WorkflowEngine,
    fake: Fake,
    clock: Arc<ManualClock>,
    store: Arc<SqliteWorkflowStore>,
    tenant: TenantId,
}

fn rig() -> Rig {
    let tenant = TenantId::new("local").unwrap();
    let store = Arc::new(SqliteWorkflowStore::open_in_memory().unwrap());
    let fake = Fake::default();
    let clock = Arc::new(ManualClock::new(Timestamp(1_700_000_000_000)));
    let engine = WorkflowEngine::new(store.clone(), Arc::new(fake.clone()), clock.clone(), tenant.clone());
    Rig { engine, fake, clock, store, tenant }
}

impl Rig {
    /// A second engine on the same store: what a restart looks like.
    fn restarted(&self) -> WorkflowEngine {
        WorkflowEngine::new(self.store.clone(), Arc::new(self.fake.clone()), self.clock.clone(), self.tenant.clone())
    }

    async fn publish(&self, spec: WorkflowSpec) -> WorkflowId {
        let workflow = self
            .store
            .create_workflow(&self.tenant, WorkflowSlug::new("flow").unwrap(), spec, Timestamp(1))
            .await
            .unwrap();
        self.store
            .publish(&self.tenant, workflow.id, None, Some(zorix_core::Confirm { by: "test".into() }), Timestamp(2))
            .await
            .unwrap();
        workflow.id
    }

    async fn run(&self, id: zorix_core::WorkflowRunId) -> zorix_core::WorkflowRun {
        self.store.run(&self.tenant, id).await.unwrap().unwrap()
    }
}

fn step(id: &str, kind: StepKind) -> Step {
    Step { id: id.into(), title: format!("Step {id}"), x: 0, y: 0, kind }
}

fn agent(id: &str, prompt: &str) -> Step {
    step(
        id,
        StepKind::Agent {
            agent: AgentSlug::new("researcher").unwrap(),
            prompt: prompt.into(),
            version: None,
            model: None,
            max_cost_micros: None,
        },
    )
}

fn edge(from: &str, to: &str, label: Option<&str>) -> Edge {
    Edge { from: from.into(), to: to.into(), label: label.map(String::from) }
}

fn spec(steps: Vec<Step>, edges: Vec<Edge>) -> WorkflowSpec {
    let mut spec = WorkflowSpec::blank("Flow");
    spec.steps.extend(steps);
    spec.edges.extend(edges);
    spec
}

fn approval(id: &str, timeout: Option<u64>, then: Then) -> Step {
    step(
        id,
        StepKind::Approval {
            message: "Go ahead with {{steps.triage.output.text}}?".into(),
            show: vec!["steps.triage.output.text".into()],
            timeout_ms: timeout,
            then,
            remind: None,
        },
    )
}

fn post(id: &str, ask: Ask) -> Step {
    step(
        id,
        StepKind::Send {
            service: Service::Slack,
            connection: None,
            channel: "#incidents".into(),
            text: "{{steps.triage.output.text}}".into(),
            ask,
        },
    )
}

fn state(run: &zorix_core::WorkflowRun, id: &str) -> StepState {
    run.steps[id].state
}

/// start → triage (agent) → high? ─yes→ approve → post
///                                 └no→ (nothing)
fn incident() -> WorkflowSpec {
    spec(
        vec![
            agent("triage", "Triage: {{trigger.payload.title}}"),
            step("high", StepKind::Condition { expr: "trigger.payload.severity >= 3".into() }),
            approval("approve", Some(2 * 60 * 60 * 1000), Then::Refuse),
            post("post", Ask::Always),
        ],
        vec![
            edge("start", "triage", None),
            edge("triage", "high", None),
            edge("high", "approve", Some("yes")),
            edge("approve", "post", None),
            // The "no" branch has to lead somewhere to be a valid condition.
            edge("high", "post", Some("no")),
        ],
    )
}

#[tokio::test]
async fn agent_then_condition_then_approval_then_action_in_order() {
    let rig = rig();
    let id = rig.publish(incident()).await;
    let run =
        rig.engine.start(StartRequest::manual(id, json!({"title": "Checkout fails", "severity": 4}))).await.unwrap();

    // The agent step started with its prompt filled in, and linked back to the run.
    assert_eq!(rig.fake.started(), 1);
    let start = rig.fake.start_of(0);
    assert_eq!(start.prompt, "Triage: Checkout fails");
    assert_eq!(start.metadata["workflow_step"], "triage");
    assert_eq!(start.metadata["workflow_run"], run.id.to_string());
    assert_eq!(run.state, RunState::Running);
    assert_eq!(state(&run, "triage"), StepState::Running);
    assert_eq!(state(&run, "high"), StepState::Pending);

    // Still running: advancing again changes nothing and starts nothing twice.
    rig.engine.advance(run.id).await.unwrap();
    assert_eq!(rig.fake.started(), 1);

    rig.fake.finish(0, "High: the EU gateway times out", 210_000);
    let run = rig.engine.advance(run.id).await.unwrap();
    assert_eq!(state(&run, "triage"), StepState::Done);
    assert_eq!(run.steps["triage"].note.as_deref(), Some("3 steps · $0.21"));
    assert_eq!(state(&run, "high"), StepState::Done);
    assert_eq!(run.steps["high"].note.as_deref(), Some("yes"));
    assert_eq!(state(&run, "approve"), StepState::Waiting);
    assert_eq!(run.state, RunState::Waiting);
    assert_eq!(run.cost_micros, 210_000);

    // The approval shows what the step chose to show.
    let waits = rig.store.pending_waits(&rig.tenant).await.unwrap();
    assert_eq!(waits.len(), 1);
    let wait = &waits[0];
    assert_eq!(wait.kind, WaitKind::Approval);
    assert_eq!(wait.message, "Go ahead with High: the EU gateway times out?");
    assert_eq!(wait.shown["steps.triage.output.text"], "High: the EU gateway times out");

    // "no" branch was not taken, but "post" still waits for the approval.
    assert_eq!(state(&run, "post"), StepState::Pending);

    let decision =
        Decision { fingerprint: wait.fingerprint.clone(), verdict: Verdict::Approve, by: "luis".into(), note: None };
    rig.engine.decide(wait.id, decision).await.unwrap();
    let run = rig.run(run.id).await;
    assert_eq!(state(&run, "approve"), StepState::Done);
    // The action asks first by default: a new wait for the call, bound to its arguments.
    assert_eq!(state(&run, "post"), StepState::Waiting);
    let waits = rig.store.pending_waits(&rig.tenant).await.unwrap();
    assert_eq!(waits[0].kind, WaitKind::Action);
    assert_eq!(waits[0].shown["tool"], "slack.send_message");
    assert_eq!(waits[0].shown["arguments"]["text"], "High: the EU gateway times out");
    assert!(rig.fake.calls().is_empty(), "nothing is sent before a person agrees");

    let decision = Decision {
        fingerprint: waits[0].fingerprint.clone(),
        verdict: Verdict::Approve,
        by: "luis".into(),
        note: None,
    };
    rig.engine.decide(waits[0].id, decision).await.unwrap();
    let run = rig.run(run.id).await;
    assert_eq!(state(&run, "post"), StepState::Done);
    assert_eq!(run.state, RunState::Completed);
    assert_eq!(rig.fake.calls().len(), 1);
    assert_eq!(rig.fake.calls()[0].arguments["channel"], "#incidents");
}

#[tokio::test]
async fn the_branch_not_taken_is_skipped_and_the_run_still_completes() {
    let rig = rig();
    let id = rig.publish(incident()).await;
    let run = rig.engine.start(StartRequest::manual(id, json!({"title": "Typo", "severity": 1}))).await.unwrap();
    rig.fake.finish(0, "Low", 0);
    let run = rig.engine.advance(run.id).await.unwrap();
    assert_eq!(run.steps["high"].note.as_deref(), Some("no"));
    assert_eq!(state(&run, "approve"), StepState::Skipped);
    // "post" has one live input (the "no" arrow), so it is reached and asks first.
    assert_eq!(state(&run, "post"), StepState::Waiting);
}

#[tokio::test]
async fn parallel_branches_join_only_when_both_are_done() {
    let rig = rig();
    let s = spec(
        vec![
            agent("a", "A"),
            agent("b", "B"),
            agent("join", "Combine {{steps.a.output.text}} and {{steps.b.output.text}}"),
        ],
        vec![edge("start", "a", None), edge("start", "b", None), edge("a", "join", None), edge("b", "join", None)],
    );
    let id = rig.publish(s).await;
    let run = rig.engine.start(StartRequest::manual(id, json!({}))).await.unwrap();
    assert_eq!(rig.fake.started(), 2, "both branches start together");
    rig.fake.finish(0, "first", 0);
    let run = rig.engine.advance(run.id).await.unwrap();
    assert_eq!(rig.fake.started(), 2, "the join waits for the other branch");
    assert_eq!(state(&run, "join"), StepState::Pending);
    rig.fake.finish(1, "second", 0);
    rig.engine.advance(run.id).await.unwrap();
    assert_eq!(rig.fake.started(), 3);
    assert_eq!(rig.fake.start_of(2).prompt, "Combine first and second");
}

#[tokio::test]
async fn a_failed_step_stops_the_rest_and_cancels_what_is_running() {
    let rig = rig();
    let s = spec(
        vec![agent("a", "A"), agent("b", "B"), agent("after", "x")],
        vec![edge("start", "a", None), edge("start", "b", None), edge("a", "after", None)],
    );
    let id = rig.publish(s).await;
    let run = rig.engine.start(StartRequest::manual(id, json!({}))).await.unwrap();
    rig.fake.fail(0, "The model refused");
    let run = rig.engine.advance(run.id).await.unwrap();
    assert_eq!(run.state, RunState::Failed);
    assert!(run.error.as_deref().unwrap().contains("Step a: The model refused"), "{:?}", run.error);
    assert_eq!(state(&run, "a"), StepState::Failed);
    assert_eq!(state(&run, "b"), StepState::Skipped);
    assert_eq!(state(&run, "after"), StepState::Skipped);
    assert_eq!(rig.fake.0.lock().cancelled.len(), 1, "the other branch was cancelled");
    assert!(run.finished_at.is_some());
}

#[tokio::test]
async fn a_template_that_reads_nothing_fails_the_step_instead_of_sending_an_empty_message() {
    let rig = rig();
    let s = spec(vec![agent("a", "Look at {{trigger.payload.missing}}")], vec![edge("start", "a", None)]);
    let id = rig.publish(s).await;
    let run = rig.engine.start(StartRequest::manual(id, json!({}))).await.unwrap();
    assert_eq!(run.state, RunState::Failed);
    assert!(run.error.as_deref().unwrap().contains("trigger.payload.missing"));
    assert_eq!(rig.fake.started(), 0);
}

#[tokio::test]
async fn an_agent_waiting_for_a_person_shows_the_step_as_waiting() {
    let rig = rig();
    let s = spec(vec![agent("a", "A")], vec![edge("start", "a", None)]);
    let id = rig.publish(s).await;
    let run = rig.engine.start(StartRequest::manual(id, json!({}))).await.unwrap();
    rig.fake.wait_for_person(0);
    let run = rig.engine.advance(run.id).await.unwrap();
    assert_eq!(state(&run, "a"), StepState::Waiting);
    assert_eq!(run.steps["a"].note.as_deref(), Some("waiting for a person"));
    assert_eq!(run.state, RunState::Waiting);
    rig.fake.finish(0, "done", 5);
    let run = rig.engine.advance(run.id).await.unwrap();
    assert_eq!(run.state, RunState::Completed);
}

#[tokio::test]
async fn the_budget_narrows_what_each_step_may_spend_and_stops_the_run_when_used() {
    let rig = rig();
    let mut s = spec(
        vec![
            agent("a", "A"),
            step(
                "b",
                StepKind::Agent {
                    agent: AgentSlug::new("researcher").unwrap(),
                    prompt: "B".into(),
                    version: None,
                    model: None,
                    max_cost_micros: Some(900_000),
                },
            ),
            agent("c", "C"),
        ],
        vec![edge("start", "a", None), edge("a", "b", None), edge("b", "c", None)],
    );
    s.budget_micros = Some(1_000_000);
    let id = rig.publish(s).await;
    let run = rig.engine.start(StartRequest::manual(id, json!({}))).await.unwrap();
    assert_eq!(rig.fake.start_of(0).max_cost_micros, Some(1_000_000));
    rig.fake.finish(0, "x", 600_000);
    rig.engine.advance(run.id).await.unwrap();
    // 400k left: the step asked for up to 900k, so it gets the smaller.
    assert_eq!(rig.fake.start_of(1).max_cost_micros, Some(400_000));
    rig.fake.finish(1, "y", 400_000);
    let run = rig.engine.advance(run.id).await.unwrap();
    assert_eq!(run.state, RunState::Failed);
    assert!(run.error.as_deref().unwrap().contains("budget is used up"), "{:?}", run.error);
    assert_eq!(run.cost_micros, 1_000_000);
}

#[tokio::test]
async fn an_unattended_action_runs_without_asking_but_a_test_run_always_asks() {
    let rig = rig();
    let s = spec(vec![post("post", Ask::Never)], vec![edge("start", "post", None)]);
    let mut s = s;
    s.steps[1].kind = StepKind::Send {
        service: Service::Slack,
        connection: None,
        channel: "#ops".into(),
        text: "hello".into(),
        ask: Ask::Never,
    };
    let id = rig.publish(s).await;
    let run = rig.engine.start(StartRequest::manual(id, json!({}))).await.unwrap();
    assert_eq!(run.state, RunState::Completed);
    assert_eq!(rig.fake.calls().len(), 1);

    let mut test = StartRequest::manual(id, json!({}));
    test.test = true;
    let run = rig.engine.start(test).await.unwrap();
    assert!(run.test);
    assert_eq!(run.version, None);
    assert_eq!(state(&run, "post"), StepState::Waiting, "a test asks before the action");
    assert_eq!(rig.fake.calls().len(), 1, "and sent nothing");
}

#[tokio::test]
async fn a_failing_action_fails_the_step_and_its_error_is_kept() {
    let rig = rig();
    rig.fake.0.lock().call_error = Some("Slack refused the request (channel_not_found).".into());
    let mut s = spec(vec![post("post", Ask::Never)], vec![edge("start", "post", None)]);
    s.steps[1].kind = StepKind::Send {
        service: Service::Slack,
        connection: None,
        channel: "#nowhere".into(),
        text: "hi".into(),
        ask: Ask::Never,
    };
    let id = rig.publish(s).await;
    let run = rig.engine.start(StartRequest::manual(id, json!({}))).await.unwrap();
    assert_eq!(run.state, RunState::Failed);
    assert!(run.error.as_deref().unwrap().contains("channel_not_found"));
}

#[tokio::test]
async fn an_answer_must_name_what_the_person_was_shown_and_counts_once() {
    let rig = rig();
    let s = spec(
        vec![step(
            "q",
            StepKind::Question {
                question: "Which environment?".into(),
                default: None,
                timeout_ms: None,
                then: Then::Refuse,
            },
        )],
        vec![edge("start", "q", None)],
    );
    let id = rig.publish(s).await;
    let run = rig.engine.start(StartRequest::manual(id, json!({}))).await.unwrap();
    let wait = rig.store.pending_waits(&rig.tenant).await.unwrap().remove(0);
    assert_eq!(wait.kind, WaitKind::Question);

    let stale = Decision {
        fingerprint: "not-what-was-shown".into(),
        verdict: Verdict::Answer("prod".into()),
        by: "luis".into(),
        note: None,
    };
    assert!(rig.engine.decide(wait.id, stale).await.unwrap_err().to_string().contains("has changed"));
    let wrong_kind =
        Decision { fingerprint: wait.fingerprint.clone(), verdict: Verdict::Approve, by: "luis".into(), note: None };
    assert!(rig.engine.decide(wait.id, wrong_kind).await.is_err());
    let empty = Decision {
        fingerprint: wait.fingerprint.clone(),
        verdict: Verdict::Answer("  ".into()),
        by: "luis".into(),
        note: None,
    };
    assert!(rig.engine.decide(wait.id, empty).await.is_err());

    let right = Decision {
        fingerprint: wait.fingerprint.clone(),
        verdict: Verdict::Answer("staging".into()),
        by: "luis".into(),
        note: None,
    };
    rig.engine.decide(wait.id, right.clone()).await.unwrap();
    assert!(rig.engine.decide(wait.id, right).await.unwrap_err().to_string().contains("already answered"));
    let run = rig.run(run.id).await;
    assert_eq!(run.state, RunState::Completed);
    assert_eq!(run.steps["q"].output.as_ref().unwrap()["answer"], "staging");
}

#[tokio::test]
async fn a_refusal_fails_the_step_and_says_who() {
    let rig = rig();
    let s = spec(vec![approval("ok", None, Then::Refuse).clone()], vec![edge("start", "ok", None)]);
    let mut s = s;
    s.steps[1].kind =
        StepKind::Approval { message: "Go?".into(), show: vec![], timeout_ms: None, then: Then::Refuse, remind: None };
    let id = rig.publish(s).await;
    let run = rig.engine.start(StartRequest::manual(id, json!({}))).await.unwrap();
    let wait = rig.store.pending_waits(&rig.tenant).await.unwrap().remove(0);
    let no = Decision {
        fingerprint: wait.fingerprint.clone(),
        verdict: Verdict::Refuse,
        by: "luis".into(),
        note: Some("too risky".into()),
    };
    rig.engine.decide(wait.id, no).await.unwrap();
    let run = rig.run(run.id).await;
    assert_eq!(run.state, RunState::Failed);
    assert!(run.error.as_deref().unwrap().contains("Refused by luis (too risky)"), "{:?}", run.error);
}

#[tokio::test]
async fn a_time_out_refuses_or_goes_on_as_the_step_says() {
    for (then, expected) in [(Then::Refuse, RunState::Failed), (Then::Continue, RunState::Completed)] {
        let rig = rig();
        let mut s = spec(vec![approval("ok", Some(60 * 60 * 1000), then)], vec![edge("start", "ok", None)]);
        s.steps[1].kind = StepKind::Approval {
            message: "Go?".into(),
            show: vec![],
            timeout_ms: Some(60 * 60 * 1000),
            then,
            remind: None,
        };
        let id = rig.publish(s).await;
        let run = rig.engine.start(StartRequest::manual(id, json!({}))).await.unwrap();
        assert_eq!(run.state, RunState::Waiting);
        rig.clock.advance(59 * 60 * 1000);
        assert_eq!(rig.engine.advance(run.id).await.unwrap().state, RunState::Waiting, "not yet");
        rig.clock.advance(2 * 60 * 1000);
        let run = rig.engine.advance(run.id).await.unwrap();
        assert_eq!(run.state, expected, "then = {then:?}");
        assert_eq!(
            rig.store.wait(&rig.tenant, run.steps["ok"].wait.unwrap()).await.unwrap().unwrap().status,
            WaitStatus::TimedOut
        );
    }
}

#[tokio::test]
async fn a_question_that_times_out_can_take_its_default() {
    let rig = rig();
    let s = spec(
        vec![step(
            "q",
            StepKind::Question {
                question: "Which?".into(),
                default: Some("staging".into()),
                timeout_ms: Some(120_000),
                then: Then::Continue,
            },
        )],
        vec![edge("start", "q", None)],
    );
    let id = rig.publish(s).await;
    let run = rig.engine.start(StartRequest::manual(id, json!({}))).await.unwrap();
    rig.clock.advance(121_000);
    let run = rig.engine.advance(run.id).await.unwrap();
    assert_eq!(run.state, RunState::Completed);
    assert_eq!(run.steps["q"].output.as_ref().unwrap()["answer"], "staging");
    assert_eq!(run.steps["q"].output.as_ref().unwrap()["timed_out"], true);
}

#[tokio::test]
async fn a_wait_step_is_a_durable_timer() {
    let rig = rig();
    let s = spec(
        vec![step("pause", StepKind::Wait { for_ms: Some(10 * 60 * 1000), until: None }), agent("after", "go")],
        vec![edge("start", "pause", None), edge("pause", "after", None)],
    );
    let id = rig.publish(s).await;
    let run = rig.engine.start(StartRequest::manual(id, json!({}))).await.unwrap();
    assert_eq!(state(&run, "pause"), StepState::Waiting);
    assert_eq!(rig.fake.started(), 0);
    rig.clock.advance(5 * 60 * 1000);
    rig.engine.advance(run.id).await.unwrap();
    assert_eq!(rig.fake.started(), 0);
    rig.clock.advance(6 * 60 * 1000);
    rig.engine.advance(run.id).await.unwrap();
    assert_eq!(rig.fake.started(), 1, "the step after the timer starts when it ends");
}

#[tokio::test]
async fn a_restart_in_the_middle_of_an_approval_resumes_from_the_store() {
    let rig = rig();
    let id = rig.publish(incident()).await;
    let run = rig.engine.start(StartRequest::manual(id, json!({"title": "Down", "severity": 5}))).await.unwrap();
    rig.fake.finish(0, "High", 0);
    rig.engine.advance(run.id).await.unwrap();
    let wait = rig.store.pending_waits(&rig.tenant).await.unwrap().remove(0);

    // A new engine, as after a restart: nothing in memory, everything in the store.
    let again = rig.restarted();
    assert_eq!(again.recover().await.unwrap(), 0);
    let run = rig.run(run.id).await;
    assert_eq!(state(&run, "approve"), StepState::Waiting);
    let decision = Decision { fingerprint: wait.fingerprint, verdict: Verdict::Approve, by: "luis".into(), note: None };
    again.decide(wait.id, decision).await.unwrap();
    assert_eq!(state(&rig.run(run.id).await, "post"), StepState::Waiting);
}

#[tokio::test]
async fn an_action_that_was_in_flight_when_the_app_stopped_is_not_repeated_but_asked_about() {
    let rig = rig();
    let mut s = spec(vec![post("post", Ask::Never)], vec![edge("start", "post", None)]);
    s.steps[1].kind = StepKind::Send {
        service: Service::Slack,
        connection: None,
        channel: "#ops".into(),
        text: "hello".into(),
        ask: Ask::Never,
    };
    let id = rig.publish(s).await;
    let run = rig.engine.start(StartRequest::manual(id, json!({}))).await.unwrap();
    assert_eq!(run.state, RunState::Completed);

    // Simulate the crash: the same run, put back to the moment after "running" was written and before the result was.
    let mut crashed = rig.run(run.id).await;
    let revision = crashed.revision;
    crashed.state = RunState::Running;
    crashed.finished_at = None;
    let record = crashed.steps.get_mut("post").unwrap();
    record.state = StepState::Running;
    record.output = None;
    record.finished_at = None;
    assert!(rig.store.put_run(&rig.tenant, &crashed, Some(revision)).await.unwrap());
    let calls_before = rig.fake.calls().len();

    let again = rig.restarted();
    assert_eq!(again.recover().await.unwrap(), 1);
    assert_eq!(rig.fake.calls().len(), calls_before, "nothing was sent again");
    let run = rig.run(run.id).await;
    assert_eq!(state(&run, "post"), StepState::Waiting);
    let wait = rig.store.pending_waits(&rig.tenant).await.unwrap().remove(0);
    assert!(wait.message.contains("may already have happened"));
    assert_eq!(wait.shown["interrupted"], true);
}

#[tokio::test]
async fn cancelling_a_run_cancels_what_is_going_and_closes_the_waits() {
    let rig = rig();
    let id = rig.publish(incident()).await;
    let run = rig.engine.start(StartRequest::manual(id, json!({"title": "Down", "severity": 5}))).await.unwrap();
    let run = rig.engine.cancel(run.id, "luis").await.unwrap();
    assert_eq!(run.state, RunState::Cancelled);
    assert_eq!(rig.fake.0.lock().cancelled.len(), 1);
    assert_eq!(state(&run, "triage"), StepState::Skipped);
    assert!(rig.engine.advance(run.id).await.unwrap().state == RunState::Cancelled, "a cancelled run stays cancelled");

    let run2 = rig.engine.start(StartRequest::manual(id, json!({"title": "Down", "severity": 5}))).await.unwrap();
    rig.fake.finish(1, "High", 0);
    rig.engine.advance(run2.id).await.unwrap();
    assert_eq!(rig.store.pending_waits(&rig.tenant).await.unwrap().len(), 1);
    rig.engine.cancel(run2.id, "luis").await.unwrap();
    assert!(rig.store.pending_waits(&rig.tenant).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_loop_runs_each_item_up_to_eight_and_collects_the_answers() {
    let rig = rig();
    let looped = step(
        "each",
        StepKind::Loop {
            over: "{{trigger.payload.items}}".into(),
            agent: AgentSlug::new("researcher").unwrap(),
            prompt: "Check {{item}} ({{index}})".into(),
        },
    );
    let s = spec(vec![looped], vec![edge("start", "each", None)]);
    let id = rig.publish(s).await;
    let run = rig.engine.start(StartRequest::manual(id, json!({"items": ["a", "b", "c"]}))).await.unwrap();
    assert_eq!(rig.fake.started(), 3);
    assert_eq!(rig.fake.start_of(1).prompt, "Check b (1)");
    rig.fake.finish(0, "A!", 10);
    rig.fake.finish(1, "B!", 10);
    assert_eq!(rig.engine.advance(run.id).await.unwrap().state, RunState::Running, "one item is still going");
    rig.fake.finish(2, "C!", 10);
    let run = rig.engine.advance(run.id).await.unwrap();
    assert_eq!(run.state, RunState::Completed);
    let items = run.steps["each"].output.as_ref().unwrap()["items"].as_array().unwrap().clone();
    assert_eq!(items.iter().map(|i| i["text"].as_str().unwrap()).collect::<Vec<_>>(), vec!["A!", "B!", "C!"]);
    assert_eq!(run.cost_micros, 30);

    let too_many =
        rig.engine.start(StartRequest::manual(id, json!({"items": (0..9).collect::<Vec<_>>()}))).await.unwrap();
    assert_eq!(too_many.state, RunState::Failed);
    assert!(too_many.error.as_deref().unwrap().contains("8 at most"));
}

#[tokio::test]
async fn only_a_published_workflow_runs_and_a_chain_is_cut() {
    let rig = rig();
    let workflow = rig
        .store
        .create_workflow(&rig.tenant, WorkflowSlug::new("draft-only").unwrap(), WorkflowSpec::blank("x"), Timestamp(1))
        .await
        .unwrap();
    let error = rig.engine.start(StartRequest::manual(workflow.id, json!({}))).await.unwrap_err().to_string();
    assert!(error.contains("no published version"), "{error}");

    let id = rig.publish(spec(vec![agent("a", "x")], vec![edge("start", "a", None)])).await;
    let mut deep = StartRequest::manual(id, json!({}));
    deep.chain = 5;
    assert!(rig.engine.start(deep).await.unwrap_err().to_string().contains("in a row"));
}

#[tokio::test]
async fn a_reminder_goes_out_once_and_the_wait_stays_open() {
    let rig = rig();
    let mut s = spec(vec![approval("ok", Some(2 * 60 * 60 * 1000), Then::Refuse)], vec![edge("start", "ok", None)]);
    s.steps[1].kind = StepKind::Approval {
        message: "Go?".into(),
        show: vec![],
        timeout_ms: Some(2 * 60 * 60 * 1000),
        then: Then::Refuse,
        remind: Some(Remind { after_ms: 30 * 60 * 1000, connection: "acme".into(), channel: "#on-call".into() }),
    };
    let id = rig.publish(s).await;
    let run = rig.engine.start(StartRequest::manual(id, json!({}))).await.unwrap();
    rig.clock.advance(29 * 60 * 1000);
    rig.engine.advance(run.id).await.unwrap();
    assert!(rig.fake.calls().is_empty());
    rig.clock.advance(2 * 60 * 1000);
    rig.engine.advance(run.id).await.unwrap();
    rig.engine.advance(run.id).await.unwrap();
    let calls = rig.fake.calls();
    assert_eq!(calls.len(), 1, "one reminder, however often the run is looked at");
    assert_eq!(calls[0].arguments["channel"], "#on-call");
    assert_eq!(calls[0].arguments["connection"], "acme");
    assert_eq!(rig.engine.advance(run.id).await.unwrap().state, RunState::Waiting);
}
