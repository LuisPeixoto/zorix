//! An agent's own memory (ADR-0026): only that agent's runs see it, a person still approves everything, and an agent without
//! memory of its own can neither read nor write it.

use std::sync::Arc;

use serde_json::json;
use zorix_core::{
    Agent, AgentBinding, AgentId, AgentSlug, AgentSpec, AgentVersion, ApprovalDecision, ApprovalStatus, ConversationId,
    Error, Execution, ExecutionEngine, ExecutionId, ExecutionState as S, Grants, MemoryFilter, MemoryStore,
    NewExecution, Permission, Scope, TaskRequest, TenantId, Timestamp, Visibility,
};
use zorix_memory::MemoryStack;
use zorix_runtime::{Engine, EngineConfig, InMemoryToolRegistry};
use zorix_sim::{ScriptedModel, Turn};
use zorix_store::{SqliteMemoryStore, SqliteRepository};

struct Env {
    _dir: tempfile::TempDir,
    store: Arc<SqliteMemoryStore>,
    engine: Engine,
}

fn env(turns: Vec<Turn>) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zorix.sqlite");
    let repository = Arc::new(SqliteRepository::open(&path).unwrap());
    let store = Arc::new(SqliteMemoryStore::open(&path).unwrap());
    let stack = MemoryStack::new(store.clone(), repository.clone(), None);
    let registry = Arc::new(InMemoryToolRegistry::new());
    stack.register_tools(&*registry).unwrap();
    let engine = stack
        .install(Engine::builder(repository, Arc::new(ScriptedModel::new(turns))).tools(registry))
        .config(EngineConfig::default())
        .build();
    Env { _dir: dir, store, engine }
}

fn agent(slug: &str, own_memory: bool) -> AgentBinding {
    let slug = AgentSlug::new(slug).unwrap();
    let mut spec = AgentSpec::new(slug.clone(), slug.as_str(), "sim");
    spec.agent_memory = own_memory;
    let record = Agent {
        id: AgentId::new(),
        tenant: TenantId::local(),
        slug,
        current_version: 1,
        archived: false,
        created_at: Timestamp(1),
        updated_at: Timestamp(1),
    };
    let version =
        AgentVersion { agent: record.id, version: 1, hash: spec.hash(), spec, note: None, created_at: Timestamp(1) };
    AgentBinding::of(&record, &version)
}

fn run(prompt: &str, binding: Option<&AgentBinding>) -> NewExecution {
    NewExecution {
        conversation: Some(ConversationId::new("chat").unwrap()),
        request: TaskRequest { prompt: prompt.into(), model: "sim".into(), ..TaskRequest::default() },
        grants: Grants::none()
            .with(Permission::use_tool("memory.remember"))
            .with(Permission::use_tool("memory.search")),
        agent: binding.cloned(),
        ..NewExecution::default()
    }
}

async fn settle(engine: &Engine, id: ExecutionId) -> Execution {
    tokio::time::timeout(std::time::Duration::from_secs(10), engine.settle(&TenantId::local(), id))
        .await
        .unwrap()
        .unwrap()
}

async fn start(env: &Env, new: NewExecution) -> Execution {
    let started = env.engine.submit(new).await.unwrap();
    settle(&env.engine, started.id).await
}

async fn approve(engine: &Engine, id: ExecutionId, automatic: bool) -> Result<Execution, Error> {
    let detail = engine.detail(&TenantId::local(), id).await.unwrap();
    let approval = detail.approvals.into_iter().find(|a| a.status == ApprovalStatus::Pending).expect("pending");
    let decision = ApprovalDecision {
        approve: true,
        decided_by: if automatic { "cli:auto".into() } else { "luis".into() },
        note: None,
        call_hash: Some(approval.call_hash),
        automatic,
    };
    engine.decide(&TenantId::local(), id, approval.id, decision).await
}

fn remember(content: &str) -> Turn {
    Turn::call("memory.remember", json!({"content": content, "kind": "fact", "scope": "agent"}))
}

fn search() -> Turn {
    Turn::call("memory.search", json!({"query": "mauve"}))
}

async fn tool_result(env: &Env, id: ExecutionId) -> serde_json::Value {
    let detail = env.engine.detail(&TenantId::local(), id).await.unwrap();
    detail.tool_calls.iter().find(|c| c.tool == "memory.search").and_then(|c| c.result.clone()).unwrap_or(json!(null))
}

#[tokio::test]
async fn an_agent_remembers_for_itself_with_a_persons_approval_and_only_it_sees_that() {
    let scout = agent("scout", true);
    let env = env(vec![
        remember("The team's favourite colour is mauve."),
        Turn::Say("Noted.".into()),
        // The same agent, later.
        search(),
        Turn::Say("It is mauve.".into()),
        // Another agent, and a run that is no agent's.
        search(),
        Turn::Say("I know nothing.".into()),
        search(),
        Turn::Say("Nor do I.".into()),
    ]);

    let waiting = start(&env, run("Remember the colour", Some(&scout))).await;
    assert_eq!(waiting.state, S::WaitingForApproval, "the agent only proposes");
    assert!(
        env.store
            .all_memory(&TenantId::local(), &MemoryFilter::default(), Timestamp(i64::MAX / 2))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        matches!(approve(&env.engine, waiting.id, true).await, Err(Error::InvalidInput(m)) if m.contains("by a person")),
        "no automatic mode"
    );
    approve(&env.engine, waiting.id, false).await.unwrap();
    assert_eq!(settle(&env.engine, waiting.id).await.state, S::Completed);

    let stored =
        env.store.all_memory(&TenantId::local(), &MemoryFilter::default(), Timestamp(i64::MAX / 2)).await.unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].scope, Scope::Agent(scout.agent));

    let same = start(&env, run("What colour?", Some(&scout))).await;
    assert_eq!(same.state, S::Completed);
    assert_eq!(tool_result(&env, same.id).await["count"], 1, "the agent finds its own memory");

    let other = agent("other", true);
    let b = start(&env, run("What colour?", Some(&other))).await;
    assert_eq!(tool_result(&env, b.id).await["count"], 0, "another agent does not");
    let plain = start(&env, run("What colour?", None)).await;
    assert_eq!(tool_result(&env, plain.id).await["count"], 0, "and neither does a run that no agent made");

    // The store agrees: the same question asked as each of them.
    let as_scout = Visibility { agent: Some(scout.agent), ..Visibility::tenant(TenantId::local()) };
    let as_other = Visibility { agent: Some(other.agent), ..Visibility::tenant(TenantId::local()) };
    let ask = |v: Visibility| {
        let store = env.store.clone();
        async move { store.list_memory(&v, &MemoryFilter::default(), Timestamp(i64::MAX / 2)).await.unwrap().len() }
    };
    assert_eq!((ask(as_scout).await, ask(as_other).await, ask(Visibility::tenant(TenantId::local())).await), (1, 0, 0));
}

#[tokio::test]
async fn an_agent_without_memory_of_its_own_cannot_use_it_and_a_plain_run_cannot_ask_for_it() {
    let amnesiac = agent("amnesiac", false);
    let env =
        env(vec![remember("Something."), Turn::Say("Done.".into()), remember("Something."), Turn::Say("Done.".into())]);

    // The proposal is shown to a person first (the scope is checked when the tool runs, after approval: a known limit), and
    // then fails with a reason. Nothing is stored either way.
    for binding in [Some(&amnesiac), None] {
        let waiting = start(&env, run("Remember", binding)).await;
        assert_eq!(waiting.state, S::WaitingForApproval);
        approve(&env.engine, waiting.id, false).await.unwrap();
        assert_eq!(settle(&env.engine, waiting.id).await.state, S::Completed, "the call fails and the model answers");
        let detail = env.engine.detail(&TenantId::local(), waiting.id).await.unwrap();
        let error = detail.tool_calls[0].error.clone().unwrap_or_default();
        assert!(error.contains("memory of its own"), "{error}");
    }
    assert!(
        env.store
            .all_memory(&TenantId::local(), &MemoryFilter::default(), Timestamp(i64::MAX / 2))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn the_agents_memory_reaches_its_own_prompt_and_nobody_elses() {
    use zorix_core::{Clock, ContextBuilder, ExecutionView, MemoryKind, MemoryOrigin, NewMemory, SystemClock};
    let env = env(vec![]);
    let scout = agent("scout", true);
    let other = agent("other", true);
    env.store
        .remember(
            &TenantId::local(),
            NewMemory {
                scope: Scope::Agent(scout.agent),
                kind: MemoryKind::Fact,
                subject: None,
                content: "Reports go to the platform channel.".into(),
                valid_until: None,
                origin: MemoryOrigin::User { name: "luis".into() },
                replaces: vec![],
            },
            SystemClock.now(),
        )
        .await
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zorix.sqlite");
    let repository = Arc::new(SqliteRepository::open(&path).unwrap());
    // The same store file as `env`, so open a stack on the env's own store instead.
    let _ = repository;
    let stack = MemoryStack::new(
        env.store.clone(),
        Arc::new(SqliteRepository::open(env._dir.path().join("zorix.sqlite").as_path()).unwrap()),
        None,
    );
    let prompt_of = |binding: Option<&AgentBinding>| {
        let new = run("Where do reports go? Reports", binding);
        Execution::new(ExecutionId::new(), new, SystemClock.now())
    };
    let seen = |execution: Execution| {
        let stack = &stack;
        async move {
            let view = ExecutionView {
                execution: &execution,
                transcript: &[],
                tools: &[],
                context_budget: Some(10_000),
                token_factor: 1.0,
            };
            let request = stack.context_builder().build(&view).await.unwrap();
            request.messages.iter().any(|m| m.content.contains("Reports go to the platform channel."))
        }
    };
    assert!(seen(prompt_of(Some(&scout))).await, "its own run is told");
    assert!(!seen(prompt_of(Some(&other))).await, "another agent is not");
    assert!(!seen(prompt_of(None)).await, "a plain run is not");
    let mut forgetful = scout.clone();
    forgetful.agent_memory = false;
    assert!(!seen(prompt_of(Some(&forgetful))).await, "an agent whose memory is off is not");
}
