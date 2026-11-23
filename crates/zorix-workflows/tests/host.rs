//! The host with a manual clock: schedules fire once and on time, a fire missed while nothing ran is recorded and not run, events
//! and finished workflows start others without repeating, a chain is cut, and webhooks answer only to the right address.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use hmac::{Hmac, Mac};
use parking_lot::Mutex;
use serde_json::{Value, json};
use sha2::Sha256;
use zorix_core::{
    AgentSlug, Day, Edge, EventOn, ExecutionId, ManualClock, Result, RunState, Step, StepKind, TenantId, Timestamp,
    When, WorkflowId, WorkflowSlug, WorkflowSpec, WorkflowStore,
};
use zorix_providers::MemoryStore;
use zorix_store::SqliteWorkflowStore;
use zorix_workflows::{
    ExecutionFeed, FinishedExecution, HostConfig, Outcome, RunStart, StepLauncher, ToolCall, WorkflowEngine,
    WorkflowHost,
};

#[derive(Default)]
struct World {
    started: Vec<RunStart>,
}

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<World>>);

#[async_trait]
impl StepLauncher for Fake {
    async fn start(&self, start: RunStart) -> std::result::Result<ExecutionId, String> {
        self.0.lock().started.push(start);
        Ok(ExecutionId::new())
    }

    async fn outcome(&self, _id: ExecutionId) -> std::result::Result<Outcome, String> {
        Ok(Outcome::Completed { text: "done".into(), structured: None, cost_micros: 0, steps: 1 })
    }

    async fn cancel(&self, _id: ExecutionId, _reason: &str) -> std::result::Result<(), String> {
        Ok(())
    }

    async fn call_tool(&self, _call: &ToolCall) -> std::result::Result<Value, String> {
        Ok(json!({}))
    }
}

#[derive(Clone, Default)]
struct Feed(Arc<Mutex<Vec<FinishedExecution>>>);

#[async_trait]
impl ExecutionFeed for Feed {
    async fn finished_since(&self, since: Timestamp) -> Result<Vec<FinishedExecution>> {
        Ok(self.0.lock().iter().filter(|f| f.finished_at > since).cloned().collect())
    }
}

struct Rig {
    host: WorkflowHost,
    store: Arc<SqliteWorkflowStore>,
    clock: Arc<ManualClock>,
    feed: Feed,
    fake: Fake,
    tenant: TenantId,
}

/// 2026-11-20 07:59:30 UTC, a Friday.
const START: i64 = 1_795_161_570_000;

async fn rig(webhook: bool) -> Rig {
    let tenant = TenantId::new("local").unwrap();
    let store = Arc::new(SqliteWorkflowStore::open_in_memory().unwrap());
    let fake = Fake::default();
    let clock = Arc::new(ManualClock::new(Timestamp(START)));
    let engine = Arc::new(WorkflowEngine::new(store.clone(), Arc::new(fake.clone()), clock.clone(), tenant.clone()));
    let feed = Feed::default();
    let config = HostConfig {
        webhook: webhook.then(|| "127.0.0.1:0".parse().unwrap()),
        tick: Duration::from_secs(3600),
        tickers: Vec::new(),
    };
    let host =
        WorkflowHost::start(engine, Arc::new(feed.clone()), Arc::new(MemoryStore::default()), clock.clone(), config)
            .await
            .unwrap();
    Rig { host, store, clock, feed, fake, tenant }
}

impl Rig {
    async fn publish(&self, slug: &str, spec: WorkflowSpec, on: bool) -> WorkflowId {
        let workflow = self
            .store
            .create_workflow(&self.tenant, WorkflowSlug::new(slug).unwrap(), spec, Timestamp(1))
            .await
            .unwrap();
        self.store
            .publish(&self.tenant, workflow.id, None, Some(zorix_core::Confirm { by: "t".into() }), Timestamp(2))
            .await
            .unwrap();
        if on {
            self.store.set_enabled(&self.tenant, workflow.id, true, Timestamp(3)).await.unwrap();
        }
        self.host.refresh();
        workflow.id
    }

    async fn runs_of(&self, id: WorkflowId) -> Vec<zorix_core::WorkflowRun> {
        self.store.runs(&self.tenant, Some(id), 50).await.unwrap()
    }

    fn advance(&self, ms: u64) {
        self.clock.advance(ms);
    }
}

fn step(id: &str, kind: StepKind) -> Step {
    Step { id: id.into(), title: id.into(), x: 0, y: 0, kind }
}

fn agent_step(id: &str) -> Step {
    step(
        id,
        StepKind::Agent {
            agent: AgentSlug::new("researcher").unwrap(),
            prompt: "go".into(),
            version: None,
            model: None,
            max_cost_micros: None,
        },
    )
}

fn with_trigger(kind: StepKind) -> WorkflowSpec {
    let mut spec = WorkflowSpec::blank("W");
    spec.steps[0].kind = kind;
    spec.steps.push(agent_step("a"));
    spec.edges.push(Edge { from: "start".into(), to: "a".into(), label: None });
    spec
}

fn weekdays_at_eight() -> StepKind {
    StepKind::Schedule {
        when: When::Daily { times: vec!["08:00".into()], days: vec![Day::Mon, Day::Tue, Day::Wed, Day::Thu, Day::Fri] },
        timezone: Some("UTC".into()),
    }
}

#[tokio::test]
async fn a_schedule_fires_once_when_due_and_then_waits_for_the_next_day() {
    let rig = rig(false).await;
    let id = rig.publish("daily", with_trigger(weekdays_at_eight()), true).await;
    rig.host.beat_now().await.unwrap();
    assert!(rig.runs_of(id).await.is_empty(), "07:59:30 is before 08:00");
    let state = &rig.store.trigger_states(&rig.tenant).await.unwrap()[0];
    assert_eq!(state.next_at, Some(Timestamp(START + 30_000)));

    rig.advance(31_000);
    rig.host.beat_now().await.unwrap();
    let runs = rig.runs_of(id).await;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].fired.label, "Weekdays 08:00");
    assert_eq!(runs[0].fired.payload["scheduled_for"], "2026-11-20T08:00:00Z");
    let state = &rig.store.trigger_states(&rig.tenant).await.unwrap()[0];
    assert_eq!(state.next_at, Some(Timestamp(START + 30_000 + 3 * 24 * 3600 * 1000)), "Friday's next is Monday");
    assert!(state.last_fired.is_some());

    rig.host.beat_now().await.unwrap();
    assert_eq!(rig.runs_of(id).await.len(), 1, "beating again does not fire it again");
}

#[tokio::test]
async fn a_fire_missed_while_nothing_ran_is_recorded_and_not_run() {
    let rig = rig(false).await;
    let id = rig.publish("daily", with_trigger(weekdays_at_eight()), true).await;
    rig.host.beat_now().await.unwrap();
    // The computer was closed from 07:59 on Friday until Tuesday evening.
    rig.advance(4 * 24 * 3600 * 1000 + 12 * 3600 * 1000);
    rig.host.beat_now().await.unwrap();
    assert!(rig.runs_of(id).await.is_empty(), "a morning report is not run at night");
    let state = &rig.store.trigger_states(&rig.tenant).await.unwrap()[0];
    assert_eq!(state.missed, 3, "Friday, Monday and Tuesday were missed");
    assert!(state.last_missed.is_some());
    assert!(state.next_at.unwrap() > rig.clock_now());
}

impl Rig {
    fn clock_now(&self) -> Timestamp {
        use zorix_core::Clock;
        self.clock.now()
    }
}

#[tokio::test]
async fn a_workflow_that_is_off_does_not_fire() {
    let rig = rig(false).await;
    let id = rig.publish("daily", with_trigger(weekdays_at_eight()), false).await;
    rig.advance(10 * 60_000);
    rig.host.beat_now().await.unwrap();
    assert!(rig.runs_of(id).await.is_empty());
    assert!(rig.store.trigger_states(&rig.tenant).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_finished_workflow_starts_the_one_that_follows_it_once() {
    let rig = rig(false).await;
    let first = rig.publish("weekly-report", with_trigger(StepKind::Manual), true).await;
    let after = rig
        .publish(
            "churn-alert",
            with_trigger(StepKind::AfterWorkflow {
                workflow: WorkflowSlug::new("weekly-report").unwrap(),
                on: EventOn::Finished,
            }),
            true,
        )
        .await;
    rig.host.beat_now().await.unwrap();
    let run = rig.host.engine_for_tests().start(zorix_workflows::StartRequest::manual(first, json!({}))).await.unwrap();
    let run = rig.host.engine_for_tests().advance(run.id).await.unwrap();
    assert_eq!(run.state, RunState::Completed);
    rig.advance(1_000);
    rig.host.beat_now().await.unwrap();
    let runs = rig.runs_of(after).await;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].chain, 1);
    assert_eq!(runs[0].from_run, Some(run.id));
    assert_eq!(runs[0].fired.label, "After weekly-report");
    assert_eq!(runs[0].fired.payload["state"], "completed");
    rig.advance(1_000);
    rig.host.beat_now().await.unwrap();
    assert_eq!(rig.runs_of(after).await.len(), 1, "not started again on the next beat");
}

#[tokio::test]
async fn a_chain_of_workflows_that_start_each_other_is_cut() {
    let rig = rig(false).await;
    let x = rig.publish("x", with_trigger(StepKind::Manual), true).await;
    // Y follows X, and X also follows Y: a loop.
    let y = rig
        .publish(
            "y",
            with_trigger(StepKind::AfterWorkflow { workflow: WorkflowSlug::new("x").unwrap(), on: EventOn::Any }),
            true,
        )
        .await;
    let mut spec = with_trigger(StepKind::Manual);
    spec.steps.insert(
        1,
        step("again", StepKind::AfterWorkflow { workflow: WorkflowSlug::new("y").unwrap(), on: EventOn::Any }),
    );
    let mut x_spec = spec;
    x_spec.edges.push(Edge { from: "again".into(), to: "a".into(), label: None });
    let workflow = rig.store.workflow(&rig.tenant, x).await.unwrap().unwrap();
    rig.store.save_draft(&rig.tenant, workflow.id, x_spec, Timestamp(10)).await.unwrap();
    rig.store.publish(&rig.tenant, workflow.id, None, None, Timestamp(11)).await.unwrap();
    rig.host.refresh();
    rig.host.beat_now().await.unwrap();

    rig.host.engine_for_tests().start(zorix_workflows::StartRequest::manual(x, json!({}))).await.unwrap();
    for _ in 0..12 {
        rig.advance(1_000);
        rig.host.beat_now().await.unwrap();
    }
    let all = rig.store.runs(&rig.tenant, None, 100).await.unwrap();
    assert!(all.len() <= 6, "the loop was cut, got {} runs", all.len());
    assert!(all.iter().all(|r| r.chain <= zorix_core::MAX_CHAIN));
    assert!(rig.runs_of(y).await.len() >= 2);
}

#[tokio::test]
async fn an_agent_run_that_ended_starts_a_workflow_but_old_history_and_workflow_runs_do_not() {
    let rig = rig(false).await;
    let id = rig
        .publish(
            "on-finish",
            with_trigger(StepKind::Event { on: EventOn::Finished, agent: Some(AgentSlug::new("researcher").unwrap()) }),
            true,
        )
        .await;
    let ended = |n: &str, agent: &str, ok: bool, at: i64, from_workflow: bool| FinishedExecution {
        id: n.into(),
        agent: Some(AgentSlug::new(agent).unwrap()),
        ok,
        finished_at: Timestamp(at),
        text: "the answer".into(),
        error: String::new(),
        from_workflow,
    };
    // Finished before the host looked for the first time: history.
    rig.feed.0.lock().push(ended("old", "researcher", true, START - 1_000, false));
    rig.host.beat_now().await.unwrap();
    assert!(rig.runs_of(id).await.is_empty());

    rig.advance(10_000);
    rig.feed.0.lock().extend([
        ended("wrong-agent", "writer", true, START + 1_000, false),
        ended("failed", "researcher", false, START + 2_000, false),
        ended("by-a-workflow", "researcher", true, START + 3_000, true),
        ended("good", "researcher", true, START + 4_000, false),
    ]);
    rig.host.beat_now().await.unwrap();
    let runs = rig.runs_of(id).await;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].fired.payload["execution"], "good");
    assert_eq!(runs[0].fired.payload["text"], "the answer");
    rig.advance(1_000);
    rig.host.beat_now().await.unwrap();
    assert_eq!(rig.runs_of(id).await.len(), 1, "once");
}

// ---------------------------------------------------------------------------------------------------------------------
// Webhooks

fn hook_spec(filter: Option<&str>, signed: bool) -> WorkflowSpec {
    with_trigger(StepKind::Webhook {
        source: "GitHub".into(),
        label: Some("issue labelled".into()),
        filter: filter.map(String::from),
        signed,
    })
}

struct Reply {
    status: u16,
    body: String,
}

impl Reply {
    fn status(&self) -> u16 {
        self.status
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

/// A plain HTTP/1.1 POST over a socket: the listener is tested, not an HTTP client.
async fn post(url: &str, body: &str, signature: Option<&str>) -> Reply {
    post_with(url, body, signature, &[]).await
}

async fn post_with(url: &str, body: &str, signature: Option<&str>, extra: &[(&str, &str)]) -> Reply {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let rest = url.strip_prefix("http://").unwrap();
    let (host, path) = rest.split_once('/').unwrap();
    let mut stream = tokio::net::TcpStream::connect(host).await.unwrap();
    let mut signature = signature.map(|s| format!("X-Hub-Signature-256: {s}\r\n")).unwrap_or_default();
    for (name, value) in extra {
        signature.push_str(&format!("{name}: {value}\r\n"));
    }
    let head = format!(
        "POST /{path} HTTP/1.1\r\nHost: {host}\r\nContent-Length: {}\r\nConnection: close\r\n{signature}\r\n",
        body.len()
    );
    // The server may answer 413 and close before the whole body is sent: a failed write is fine, the answer is still read.
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(body.as_bytes()).await;
    let mut raw = Vec::new();
    let _ = stream.read_to_end(&mut raw).await;
    let text = String::from_utf8_lossy(&raw).to_string();
    let status = text.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
    Reply { status, body }
}

fn sign(secret: &str, body: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body.as_bytes());
    format!("sha256={}", mac.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect::<String>())
}

#[tokio::test]
async fn a_webhook_answers_only_to_the_right_address_and_starts_a_run_with_its_body() {
    let rig = rig(true).await;
    let id = rig.publish("incident", hook_spec(None, false), true).await;
    let addr = rig.host.webhook_addr().expect("listening");
    let token = rig.host.webhook_token(id).await.unwrap();
    assert_eq!(rig.host.webhook_token(id).await.unwrap(), token, "the same secret each time");
    let url = |slug: &str, token: &str| format!("http://{addr}/hooks/{slug}/{token}");

    assert_eq!(post(&url("incident", "wrong"), "{}", None).await.status(), 404);
    assert_eq!(post(&url("nobody", &token), "{}", None).await.status(), 404);
    assert_eq!(post(&url("incident", &token), "{\"issue\":{\"title\":\"Down\"}}", None).await.status(), 202);
    let runs = rig.runs_of(id).await;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].fired.payload["issue"]["title"], "Down");
    assert_eq!(runs[0].fired.label, "GitHub · issue labelled");

    // A body that is not JSON arrives as text.
    assert_eq!(post(&url("incident", &token), "plain words", None).await.status(), 202);
    assert_eq!(rig.runs_of(id).await[0].fired.payload["body"], "plain words");

    // A new address: the old one stops.
    let fresh = rig.host.reset_webhook_token(id).await.unwrap();
    assert_ne!(fresh, token);
    assert_eq!(post(&url("incident", &token), "{}", None).await.status(), 404);
    assert_eq!(post(&url("incident", &fresh), "{}", None).await.status(), 202);
    rig.host.shutdown().await;
}

#[tokio::test]
async fn a_workflow_that_is_off_or_unpublished_looks_like_it_does_not_exist() {
    let rig = rig(true).await;
    let id = rig.publish("incident", hook_spec(None, false), false).await;
    let token = rig.host.webhook_token(id).await.unwrap();
    let addr = rig.host.webhook_addr().unwrap();
    assert_eq!(post(&format!("http://{addr}/hooks/incident/{token}"), "{}", None).await.status(), 404);
    rig.store.set_enabled(&rig.tenant, id, true, Timestamp(5)).await.unwrap();
    assert_eq!(post(&format!("http://{addr}/hooks/incident/{token}"), "{}", None).await.status(), 202);
    rig.host.shutdown().await;
}

#[tokio::test]
async fn a_filter_ignores_what_it_does_not_want() {
    let rig = rig(true).await;
    let id = rig
        .publish("incident", hook_spec(Some("action == \"labeled\" and label.name == \"incident\""), false), true)
        .await;
    let token = rig.host.webhook_token(id).await.unwrap();
    let url = format!("http://{}/hooks/incident/{token}", rig.host.webhook_addr().unwrap());
    let other = post(&url, "{\"action\":\"labeled\",\"label\":{\"name\":\"bug\"}}", None).await;
    assert_eq!(other.status(), 200);
    assert_eq!(other.json()["ignored"], true);
    assert!(rig.runs_of(id).await.is_empty());
    assert_eq!(post(&url, "{\"action\":\"labeled\",\"label\":{\"name\":\"incident\"}}", None).await.status(), 202);
    assert_eq!(rig.runs_of(id).await.len(), 1);
    rig.host.shutdown().await;
}

#[tokio::test]
async fn a_signed_webhook_needs_a_valid_signature() {
    let rig = rig(true).await;
    let id = rig.publish("incident", hook_spec(None, true), true).await;
    let token = rig.host.webhook_token(id).await.unwrap();
    let url = format!("http://{}/hooks/incident/{token}", rig.host.webhook_addr().unwrap());
    let body = "{\"action\":\"opened\"}";
    assert_eq!(post(&url, body, None).await.status(), 401, "no signature");
    let secret = rig.host.signing_secret(id).await.unwrap();
    assert_eq!(post(&url, body, Some(&sign("not-the-secret", body))).await.status(), 401, "wrong secret");
    assert_eq!(
        post(&url, body, Some(&sign(&secret, "{\"action\":\"other\"}"))).await.status(),
        401,
        "signed a different body"
    );
    assert!(rig.runs_of(id).await.is_empty());
    assert_eq!(post(&url, body, Some(&sign(&secret, body))).await.status(), 202);
    assert_eq!(rig.runs_of(id).await.len(), 1);
    rig.host.shutdown().await;
}

#[tokio::test]
async fn a_delivery_that_arrives_twice_starts_one_run_and_a_new_id_starts_another() {
    let rig = rig(true).await;
    let id = rig.publish("incident", hook_spec(None, false), true).await;
    let token = rig.host.webhook_token(id).await.unwrap();
    let url = format!("http://{}/hooks/incident/{token}", rig.host.webhook_addr().unwrap());
    let first = [("X-GitHub-Delivery", "72d3162e-cc78-11e3-81ab-4c9367dc0958")];
    assert_eq!(post_with(&url, "{\"n\":1}", None, &first).await.status(), 202);
    let again = post_with(&url, "{\"n\":1}", None, &first).await;
    assert_eq!((again.status(), again.json()["duplicate"].clone()), (200, serde_json::json!(true)));
    // A retry may even change its body; it is the same delivery, so it still starts nothing.
    assert_eq!(post_with(&url, "{\"n\":2}", None, &first).await.json()["duplicate"], true);
    assert_eq!(rig.runs_of(id).await.len(), 1, "one delivery, one run");
    assert_eq!(post_with(&url, "{\"n\":1}", None, &[("X-GitHub-Delivery", "another")]).await.status(), 202);
    assert_eq!(rig.runs_of(id).await.len(), 2);
    // A sender with no delivery id may send the same body again and again; each one is wanted.
    assert_eq!(post(&url, "{}", None).await.status(), 202);
    assert_eq!(post(&url, "{}", None).await.status(), 202);
    assert_eq!(rig.runs_of(id).await.len(), 4);
    rig.host.shutdown().await;
}

#[tokio::test]
async fn an_unsigned_request_cannot_use_up_a_delivery_id() {
    let rig = rig(true).await;
    let id = rig.publish("incident", hook_spec(None, true), true).await;
    let token = rig.host.webhook_token(id).await.unwrap();
    let url = format!("http://{}/hooks/incident/{token}", rig.host.webhook_addr().unwrap());
    let body = "{\"action\":\"opened\"}";
    let delivery = [("X-GitHub-Delivery", "d-1")];
    assert_eq!(post_with(&url, body, None, &delivery).await.status(), 401);
    let secret = rig.host.signing_secret(id).await.unwrap();
    assert_eq!(
        post_with(&url, body, Some(&sign(&secret, body)), &delivery).await.status(),
        202,
        "the real one still goes through"
    );
    assert_eq!(post_with(&url, body, Some(&sign(&secret, body)), &delivery).await.json()["duplicate"], true);
    assert_eq!(rig.runs_of(id).await.len(), 1);
    rig.host.shutdown().await;
}

#[tokio::test]
async fn a_body_over_a_megabyte_is_refused() {
    let rig = rig(true).await;
    let id = rig.publish("incident", hook_spec(None, false), true).await;
    let token = rig.host.webhook_token(id).await.unwrap();
    let url = format!("http://{}/hooks/incident/{token}", rig.host.webhook_addr().unwrap());
    let big = "x".repeat(1024 * 1024 + 1);
    assert_eq!(post(&url, &big, None).await.status(), 413);
    assert!(rig.runs_of(id).await.is_empty());
    rig.host.shutdown().await;
}

#[tokio::test]
async fn nothing_listens_unless_asked_and_shutdown_stops_the_listener() {
    let off = rig(false).await;
    assert!(off.host.webhook_addr().is_none());
    off.host.shutdown().await;

    let on = rig(true).await;
    let addr = on.host.webhook_addr().unwrap();
    assert!(addr.ip().is_loopback());
    on.host.shutdown().await;
    assert!(tokio::net::TcpStream::connect(addr).await.is_err(), "the port is closed after shutdown");
    let _ = (on.tenant, on.fake);
}
