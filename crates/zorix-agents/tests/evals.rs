//! Evaluations through a real engine: checks pass and fail for the right reasons, writes are dropped, approvals are refused,
//! the judge grades, and what a run records is kept apart from real work.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zorix_agents::evals::{Change, EvalOptions, Evaluator, compare};
use zorix_agents::{ModelResolver, RunnerError};
use zorix_core::{
    Agent, AgentSlug, AgentSpec, AgentStore, AgentVersion, ApprovalStatus, EVAL_LABEL, EvalCase, EvalCheck, EvalJudge,
    ExecutionEngine, ModelGateway, StoredCase, TenantId, Timestamp, Tool, ToolAnnotations, ToolContext, ToolError,
    ToolRegistry, ToolSpec,
};
use zorix_runtime::{Engine, InMemoryToolRegistry, MemoryRepository};
use zorix_sim::{ScriptedModel, Turn};
use zorix_store::SqliteAgentStore;

struct Anything;

impl ModelResolver for Anything {
    fn candidates(&self, model: &str) -> Result<Vec<(Option<String>, String)>, String> {
        Ok(vec![(None, model.to_string())])
    }
}

struct Probe {
    spec: ToolSpec,
    runs: Arc<AtomicU32>,
}

#[async_trait]
impl Tool for Probe {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, _ctx: &ToolContext, _args: Value) -> Result<Value, ToolError> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"ok": true}))
    }
}

fn probe(id: &str, approval: bool) -> (Arc<Probe>, Arc<AtomicU32>) {
    let mut spec = ToolSpec::new(id, id, "A test tool.", json!({"type": "object"}));
    spec.annotations = if approval {
        ToolAnnotations { read_only: false, destructive: true, idempotent: false, open_world: false }
    } else {
        ToolAnnotations::safe_read()
    };
    spec.requires_approval = approval;
    let runs = Arc::new(AtomicU32::new(0));
    (Arc::new(Probe { spec, runs: runs.clone() }), runs)
}

struct World {
    evaluator: Evaluator,
    engine: Arc<Engine>,
    store: Arc<SqliteAgentStore>,
    agent: Agent,
    version: AgentVersion,
    reads: Arc<AtomicU32>,
    writes: Arc<AtomicU32>,
}

async fn world(turns: Vec<Turn>, judge_turns: Option<Vec<Turn>>, tweak: impl FnOnce(&mut AgentSpec)) -> World {
    let registry = Arc::new(InMemoryToolRegistry::new());
    let (read, reads) = probe("agent.read", false);
    let (write, writes) = probe("agent.write", true);
    registry.register(read).unwrap();
    registry.register(write).unwrap();
    let engine = Arc::new(
        Engine::builder(Arc::new(MemoryRepository::new()), Arc::new(ScriptedModel::new(turns))).tools(registry).build(),
    );
    let store = Arc::new(SqliteAgentStore::open_in_memory().unwrap());
    let mut spec = AgentSpec::new(AgentSlug::new("tested").unwrap(), "Tested", "sim");
    spec.tools = vec!["agent.read".into(), "agent.write".into()];
    spec.access = vec!["write:out/*".into(), "http:api.example.com".into()];
    tweak(&mut spec);
    let (agent, version) = store.create_agent(&TenantId::local(), spec, None, Timestamp(1)).await.unwrap();
    let judge: Option<Arc<dyn ModelGateway>> =
        judge_turns.map(|t| Arc::new(ScriptedModel::new(t)) as Arc<dyn ModelGateway>);
    let evaluator = Evaluator { engine: engine.clone(), store: store.clone(), resolver: Arc::new(Anything), judge };
    World { evaluator, engine, store, agent, version, reads, writes }
}

fn case(name: &str, prompt: &str, checks: Vec<EvalCheck>) -> StoredCase {
    StoredCase {
        id: zorix_core::EvalCaseId::new(),
        agent: zorix_core::AgentId::new(),
        case: EvalCase {
            name: name.into(),
            prompt: prompt.into(),
            input: None,
            tags: vec![],
            checks,
            judge: None,
            approve: vec![],
            allow_writes: false,
        },
        updated_at: Timestamp(1),
    }
}

fn contains(text: &str) -> EvalCheck {
    EvalCheck::Contains { text: text.into(), ignore_case: false }
}

fn one() -> EvalOptions {
    EvalOptions { concurrency: 1, ..EvalOptions::default() }
}

async fn run(w: &World, cases: Vec<StoredCase>, options: EvalOptions) -> zorix_core::EvalRun {
    w.evaluator.run(&w.agent, &w.version, cases, options, CancellationToken::new(), None).await.unwrap()
}

#[tokio::test]
async fn checks_pass_and_fail_for_stated_reasons_and_the_run_is_kept() {
    let w = world(vec![Turn::Say("Hello, world!".into()), Turn::Say("Hello, world!".into())], None, |_| {}).await;
    let cases = vec![
        case(
            "greets",
            "Say hello",
            vec![
                contains("Hello"),
                EvalCheck::Regex { pattern: r"^Hello, \w+!$".into() },
                EvalCheck::MaxSteps { steps: 1 },
            ],
        ),
        case(
            "mentions-moon",
            "Say hello",
            vec![contains("moon"), EvalCheck::NotContains { text: "world".into(), ignore_case: true }],
        ),
    ];
    let record = run(&w, cases, one()).await;
    assert_eq!(record.status, "finished");
    assert_eq!((record.agent, record.version), (w.agent.id, 1));
    let (good, bad) = (&record.results[0], &record.results[1]);
    assert!(good.passed, "{good:?}");
    assert!(
        good.checks.iter().all(|c| c.passed) && good.checks.len() == 4,
        "the status check is added: {:?}",
        good.checks
    );
    assert_eq!((good.steps, good.state.as_str()), (1, "completed"));
    assert!(!bad.passed);
    let failing: Vec<&str> = bad.checks.iter().filter(|c| !c.passed).map(|c| c.check.as_str()).collect();
    assert_eq!(failing.len(), 2, "{failing:?}");
    assert!(failing.iter().any(|c| c.contains("moon")) && failing.iter().any(|c| c.contains("does not contain")));

    // The run is stored with its results, and the executions carry the label that keeps them out of past work.
    let stored = w.store.eval_run(&TenantId::local(), record.id).await.unwrap().unwrap();
    assert_eq!(stored.results.len(), 2);
    let execution = w.engine.get(&TenantId::local(), good.execution.unwrap()).await.unwrap();
    assert_eq!(execution.request.metadata[EVAL_LABEL], record.id.to_string());
    assert!(execution.conversation.is_none(), "an evaluation has no conversation");
    assert_eq!(execution.memory, zorix_core::MemoryOptions::none(), "and uses no memory");
    assert!(!execution.agent.as_ref().unwrap().agent_memory);
}

#[tokio::test]
async fn tool_checks_see_what_was_called() {
    let w = world(
        vec![Turn::call("agent.read", json!({})), Turn::Say("done".into()), Turn::Say("done".into())],
        None,
        |_| {},
    )
    .await;
    let cases = vec![
        case(
            "used",
            "Read it",
            vec![
                EvalCheck::ToolUsed { pattern: "agent.*".into(), min: 1 },
                EvalCheck::ToolNotUsed { pattern: "agent.write".into() },
                EvalCheck::MaxToolCalls { calls: 1 },
            ],
        ),
        case("not-used", "Say hi", vec![EvalCheck::ToolUsed { pattern: "agent.read".into(), min: 1 }]),
    ];
    let record = run(&w, cases, one()).await;
    assert!(record.results[0].passed, "{:?}", record.results[0].checks);
    assert_eq!(record.results[0].tool_calls, 1);
    assert!(!record.results[1].passed);
    assert_eq!(w.reads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_approval_is_refused_automatically_unless_the_case_names_the_tool() {
    let w = world(
        vec![
            Turn::call("agent.write", json!({})),
            Turn::Say("I could not write.".into()),
            Turn::call("agent.write", json!({})),
            Turn::Say("Written.".into()),
        ],
        None,
        |_| {},
    )
    .await;
    let refused = case("refused", "Write something", vec![EvalCheck::ToolNotUsed { pattern: "agent.write".into() }]);
    let record = run(&w, vec![refused.clone()], one()).await;
    assert_eq!(w.writes.load(Ordering::SeqCst), 0, "the tool never ran");
    let result = &record.results[0];
    let detail = w.engine.detail(&TenantId::local(), result.execution.unwrap()).await.unwrap();
    let approval = &detail.approvals[0];
    assert_eq!(approval.status, ApprovalStatus::Denied);
    assert_eq!(approval.decided_by.as_deref(), Some("evaluation"));
    assert_eq!(approval.note.as_deref(), Some("evaluations refuse approvals"));
    assert!(!result.passed, "asking for the tool at all fails ToolNotUsed: it was requested");

    let mut allowed =
        case("allowed", "Write something", vec![EvalCheck::ToolUsed { pattern: "agent.write".into(), min: 1 }]);
    allowed.case.approve = vec!["agent.*".into()];
    let record = run(&w, vec![allowed], one()).await;
    assert!(record.results[0].passed, "{:?}", record.results[0]);
    assert_eq!(w.writes.load(Ordering::SeqCst), 1, "a case may name the tools it approves");
}

#[tokio::test]
async fn write_and_network_permissions_are_dropped_unless_allowed() {
    let w = world(vec![Turn::Say("a".into()), Turn::Say("b".into()), Turn::Say("c".into())], None, |_| {}).await;
    let plain = case("plain", "x", vec![contains("a")]);
    let mut writes = case("writes", "x", vec![contains("b")]);
    writes.case.allow_writes = true;
    let record = run(&w, vec![plain, writes], one()).await;
    let grants = |i: usize| {
        let engine = w.engine.clone();
        let id = record.results[i].execution.unwrap();
        async move { engine.get(&TenantId::local(), id).await.unwrap().grants }
    };
    use zorix_core::{Action, Permission};
    let (g0, g1) = (grants(0).await, grants(1).await);
    assert!(!g0.allows(&Permission::new(Action::Write, "workspace:out/a.txt")), "writes are dropped");
    assert!(g0.allows(&Permission::new(Action::Network, "http:api.example.com")), "the network stays");
    assert!(g1.allows(&Permission::new(Action::Write, "workspace:out/a.txt")), "unless the case allows writes");

    let offline =
        run(&w, vec![case("offline", "x", vec![contains("c")])], EvalOptions { offline: true, ..one() }).await;
    let g = w.engine.get(&TenantId::local(), offline.results[0].execution.unwrap()).await.unwrap().grants;
    assert!(!g.allows(&Permission::new(Action::Network, "http:api.example.com")), "--offline drops the network too");
    assert!(g.allows(&Permission::use_tool("agent.read")), "and tools are kept");
}

#[tokio::test]
async fn a_judge_grades_against_the_rubric_and_a_low_score_or_a_broken_reply_fails_the_case() {
    let w = world(
        vec![
            Turn::Say("A clear answer.".into()),
            Turn::Say("A muddled answer.".into()),
            Turn::Say("Another answer.".into()),
        ],
        Some(vec![
            Turn::Say(r#"{"score": 5, "reasons": "Clear."}"#.into()),
            Turn::Say(r#"{"score": 2, "reasons": "Muddled."}"#.into()),
            Turn::Say("I refuse to answer in JSON.".into()),
        ]),
        |_| {},
    )
    .await;
    let judged = |name: &str| {
        let mut c = case(name, "Explain it", vec![]);
        c.case.judge =
            Some(EvalJudge { rubric: "The answer is clear.".into(), model: "judge-model".into(), pass_score: 4 });
        c
    };
    let record = run(&w, vec![judged("clear"), judged("muddled"), judged("broken")], one()).await;
    let [clear, muddled, broken] = &record.results[..] else { panic!("three results") };
    assert!(clear.passed && clear.judge.as_ref().unwrap().score == 5, "{clear:?}");
    assert_eq!(clear.judge.as_ref().unwrap().model, "judge-model");
    assert!(!muddled.passed && muddled.judge.as_ref().unwrap().reasons == "Muddled.", "{muddled:?}");
    assert!(!broken.passed && broken.judge.is_none());
    assert!(broken.checks.iter().any(|c| c.check == "judged" && c.detail.contains("JSON")), "{:?}", broken.checks);
}

#[tokio::test]
async fn a_case_with_a_judge_but_no_judge_model_says_so() {
    let w = world(vec![Turn::Say("x".into())], None, |_| {}).await;
    let mut c = case("needs-judge", "Explain", vec![]);
    c.case.judge = Some(EvalJudge { rubric: "Good.".into(), model: "m".into(), pass_score: 3 });
    let record = run(&w, vec![c], one()).await;
    assert!(!record.results[0].passed);
    assert!(record.results[0].checks.iter().any(|c| c.detail.contains("no judge model")));
}

#[tokio::test]
async fn tags_pick_cases_an_empty_suite_is_an_error_and_cancelling_stops_the_rest() {
    let w = world(vec![Turn::Say("fast".into()), Turn::Say("never".into())], None, |_| {}).await;
    let mut quick = case("quick", "x", vec![contains("fast")]);
    quick.case.tags = vec!["smoke".into()];
    let slow = case("slow", "x", vec![contains("never")]);
    let record = run(&w, vec![quick.clone(), slow.clone()], EvalOptions { tags: vec!["smoke".into()], ..one() }).await;
    assert_eq!(record.results.len(), 1);
    assert_eq!(record.results[0].case, "quick");

    let none = w
        .evaluator
        .run(
            &w.agent,
            &w.version,
            vec![slow.clone()],
            EvalOptions { tags: vec!["nope".into()], ..one() },
            CancellationToken::new(),
            None,
        )
        .await;
    assert!(matches!(none, Err(RunnerError::Refused(m)) if m.contains("has one of the tags")));
    let empty = w.evaluator.run(&w.agent, &w.version, vec![], one(), CancellationToken::new(), None).await;
    assert!(matches!(empty, Err(RunnerError::Refused(m)) if m.contains("no evaluation cases")));

    let token = CancellationToken::new();
    token.cancel();
    let cancelled = w.evaluator.run(&w.agent, &w.version, vec![slow], one(), token, None).await.unwrap();
    assert_eq!(cancelled.status, "cancelled");
    assert!(cancelled.results[0].error.as_deref().unwrap().contains("cancelled"));
}

#[tokio::test]
async fn a_model_the_agent_may_not_use_is_a_clear_failure_for_every_case() {
    let w = world(vec![], None, |s| s.models.allowed = vec!["anthropic/*".into()]).await;
    struct Fixed;
    impl ModelResolver for Fixed {
        fn candidates(&self, _: &str) -> Result<Vec<(Option<String>, String)>, String> {
            Ok(vec![(Some("openai".into()), "gpt-5".into())])
        }
    }
    let evaluator = Evaluator { resolver: Arc::new(Fixed), ..w.evaluator.clone() };
    let record = evaluator
        .run(&w.agent, &w.version, vec![case("c", "x", vec![contains("x")])], one(), CancellationToken::new(), None)
        .await
        .unwrap();
    assert!(!record.results[0].passed);
    assert!(record.results[0].error.as_deref().unwrap().contains("may not use"), "{:?}", record.results[0]);
    assert!(record.results[0].execution.is_none(), "nothing ran");
}

#[tokio::test]
async fn two_runs_compare_case_by_case() {
    let w = world(
        vec![Turn::Say("ok".into()), Turn::Say("no".into()), Turn::Say("ok".into()), Turn::Say("ok".into())],
        None,
        |_| {},
    )
    .await;
    let cases = vec![case("a", "x", vec![contains("ok")]), case("b", "x", vec![contains("ok")])];
    let before = run(&w, cases.clone(), one()).await;
    let after = run(&w, cases, one()).await;
    assert!(before.results[0].passed && !before.results[1].passed);
    assert!(after.results[0].passed && after.results[1].passed);
    let rows = compare(&before, &after);
    assert_eq!(
        rows.iter().map(|r| (r.case.as_str(), r.change)).collect::<Vec<_>>(),
        [("a", Change::StillPasses), ("b", Change::Fixed)]
    );
    let back = compare(&after, &before);
    assert_eq!(back[1].change, Change::Broke);
    let partial = zorix_core::EvalRun { results: vec![before.results[0].clone()], ..before.clone() };
    assert_eq!(compare(&partial, &after)[1].change, Change::OnlyInOne);
}
