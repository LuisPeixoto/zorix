//! Delegation between agents through a real engine and store: who may be asked, what a child may do and spend, what it is told,
//! and how evaluations treat a whole tree (ADR-0029 to ADR-0031).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zorix_agents::{AgentAsk, AgentPlanner, ModelResolver, prepare};
use zorix_core::{
    Action, AgentSlug, AgentSpec, AgentStore, DelegateTool, EvalCase, EvalCheck, ExecutionEngine, ExecutionId,
    ExecutionState as S, FinishReason, Message, ModelError, ModelGateway, ModelRequest, ModelResponse, Permission,
    Role, TenantId, Timestamp, Tool, ToolAnnotations, ToolCallId, ToolCallRequest, ToolContext, ToolError,
    ToolRegistry, ToolSpec, Usage,
};
use zorix_runtime::{Engine, EngineConfig, InMemoryToolRegistry, MemoryRepository};
use zorix_store::SqliteAgentStore;

struct Anything;

impl ModelResolver for Anything {
    fn candidates(&self, model: &str) -> Result<Vec<(Option<String>, String)>, String> {
        Ok(match model.split_once('/') {
            Some((provider, name)) => vec![(Some(provider.to_string()), name.to_string())],
            None => vec![(None, model.to_string())],
        })
    }
}

/// A tool that asks for approval on every call.
struct Writer(ToolSpec);

#[async_trait]
impl Tool for Writer {
    fn spec(&self) -> &ToolSpec {
        &self.0
    }

    async fn call(&self, _ctx: &ToolContext, _args: Value) -> Result<Value, ToolError> {
        Ok(json!({"written": true}))
    }
}

fn writer() -> Arc<Writer> {
    let mut spec = ToolSpec::new("probe.write", "write", "Writes something.", json!({"type": "object"}));
    spec.annotations = ToolAnnotations { read_only: false, destructive: true, idempotent: false, open_world: false };
    spec.requires_approval = true;
    Arc::new(Writer(spec))
}

/// Decides by the prompt, so parallel children cannot disturb each other:
/// `ASK a: x | b: y` delegates; `WRITE` calls the writer once; anything else answers `done: <prompt>`.
struct ByPrompt;

fn say(request: &ModelRequest, text: String) -> ModelResponse {
    let usage = Usage { input_tokens: 10, output_tokens: 5, cost_micros: 100, model_calls: 1, ..Usage::default() };
    ModelResponse::new(request.model.clone(), text, vec![], usage, FinishReason::Stop)
}

fn call(request: &ModelRequest, tool: &str, arguments: Value) -> ModelResponse {
    let usage = Usage { input_tokens: 10, output_tokens: 5, cost_micros: 100, model_calls: 1, ..Usage::default() };
    let call = ToolCallRequest { id: ToolCallId(String::new()), provider_id: None, tool: tool.into(), arguments };
    ModelResponse::new(request.model.clone(), String::new(), vec![call], usage, FinishReason::ToolCalls)
}

#[async_trait]
impl ModelGateway for ByPrompt {
    async fn generate(&self, request: ModelRequest, _cancel: CancellationToken) -> Result<ModelResponse, ModelError> {
        let prompt =
            request.messages.iter().find(|m| m.role == Role::User).map(|m| m.content.clone()).unwrap_or_default();
        let result = request.messages.iter().rev().find(|m| m.role == Role::Tool).map(|m: &Message| m.content.clone());
        if let Some(list) = prompt.strip_prefix("ASK ") {
            return Ok(match result {
                Some(text) => say(&request, format!("combined: {text}")),
                None => {
                    let tasks: Vec<Value> = list
                        .split('|')
                        .map(|part| {
                            let (agent, task) = part.split_once(':').unwrap_or(("", part));
                            json!({"agent": agent.trim(), "task": task.trim()})
                        })
                        .collect();
                    call(&request, "agents.delegate", json!({ "tasks": tasks }))
                }
            });
        }
        if prompt.starts_with("WRITE") {
            return Ok(match result {
                Some(_) => say(&request, "wrote it".into()),
                None => call(&request, "probe.write", json!({})),
            });
        }
        Ok(say(&request, format!("done: {prompt}")))
    }
}

struct World {
    engine: Engine,
    store: Arc<SqliteAgentStore>,
}

fn tenant() -> TenantId {
    TenantId::local()
}

async fn make(store: &SqliteAgentStore, slug: &str, access: &[&str], configure: impl FnOnce(&mut AgentSpec)) {
    let mut spec = AgentSpec::new(AgentSlug::new(slug).unwrap(), slug.to_uppercase(), "demo");
    spec.description = format!("The {slug} agent.");
    spec.instructions = format!("You are {slug}.");
    spec.access = access.iter().map(|a| a.to_string()).collect();
    spec.tools = vec!["probe.write".into()];
    spec.limits.max_tool_calls = 12;
    configure(&mut spec);
    store.create_agent(&tenant(), spec, None, Timestamp(1)).await.unwrap();
}

async fn world() -> World {
    let store = Arc::new(SqliteAgentStore::open_in_memory().unwrap());
    let registry = Arc::new(InMemoryToolRegistry::new());
    registry.register(Arc::new(DelegateTool::new())).unwrap();
    registry.register(writer()).unwrap();
    let planner = Arc::new(AgentPlanner::new(store.clone(), Arc::new(Anything)));
    let config = EngineConfig { workers: 3, lease_ttl_ms: 1_000, ..EngineConfig::default() };
    let engine = Engine::builder(Arc::new(MemoryRepository::new()), Arc::new(ByPrompt))
        .tools(registry)
        .delegation(planner)
        .config(config)
        .build();
    World { engine, store }
}

impl World {
    async fn ask(&self, slug: &str, ask: AgentAsk) -> zorix_core::NewExecution {
        let agent = self.store.agent_by_slug(&tenant(), &AgentSlug::new(slug).unwrap()).await.unwrap().unwrap();
        let version = self.store.version(&tenant(), agent.id, agent.current_version).await.unwrap().unwrap();
        prepare(&agent, &version, ask, &*self.store, &Anything).await.unwrap()
    }

    async fn run(&self, slug: &str, prompt: &str) -> zorix_core::Execution {
        let new = self.ask(slug, AgentAsk { prompt: prompt.into(), ..AgentAsk::default() }).await;
        let created = self.engine.create(new).await.unwrap();
        self.engine.start(&tenant(), created.id).await.unwrap();
        tokio::time::timeout(Duration::from_secs(10), self.engine.settle(&tenant(), created.id))
            .await
            .expect("settles")
            .unwrap()
    }

    async fn tree(&self, id: ExecutionId) -> Vec<zorix_core::Execution> {
        self.engine.tree(&tenant(), id).await.unwrap()
    }
}

#[tokio::test]
async fn a_lead_asks_the_agents_it_may_and_each_child_is_bound_to_the_agent_that_ran_it() {
    let w = world().await;
    make(&w.store, "lead", &["agent:researcher"], |_| {}).await;
    make(&w.store, "researcher", &[], |_| {}).await;
    make(&w.store, "writer", &[], |_| {}).await;
    let done = w.run("lead", "ASK researcher: find the facts").await;
    assert_eq!(done.state, S::Completed, "{:?}", done.failure);
    assert!(done.result.as_ref().unwrap().text.contains("done: find the facts"));
    let all = w.tree(done.id).await;
    assert_eq!(all.len(), 2);
    let child = &all[1];
    let binding = child.agent.as_ref().expect("a child made by an agent records it");
    assert_eq!((binding.slug.as_str(), binding.version), ("researcher", 1));
    assert_eq!((child.depth, child.parent, child.root), (1, Some(done.id), Some(done.id)));
    assert_eq!(child.request.metadata.get("delegated_by").map(String::as_str), Some("lead"));
    assert!(child.conversation.is_none());

    // The lead may not ask an agent that is not in its access, and is told whom it may ask.
    let refused = w.run("lead", "ASK writer: write it up").await;
    assert_eq!(refused.state, S::Completed);
    let text = refused.result.unwrap().text;
    assert!(text.contains("You may not ask writer") && text.contains("You may ask: researcher"), "{text}");
    assert_eq!(w.tree(refused.id).await.len(), 1, "nothing started");
    let unknown = w.run("lead", "ASK nobody: hello").await;
    assert!(unknown.result.unwrap().text.contains("You may not ask nobody"));
}

#[tokio::test]
async fn a_child_can_never_do_more_than_its_parent() {
    let w = world().await;
    // The lead has no write access; the researcher does, so the intersection is what the child gets.
    make(&w.store, "lead", &["agent:researcher", "read:docs/*"], |s| s.tools.clear()).await;
    make(&w.store, "researcher", &["read:*", "write:out/*", "http:example.com"], |_| {}).await;
    let done = w.run("lead", "ASK researcher: look").await;
    let child = &w.tree(done.id).await[1];
    let reads = |path: &str| child.grants.allows(&zorix_core::workspace_grant(Action::Read, path).unwrap());
    assert!(reads("docs/a.md"), "both may read docs");
    assert!(!reads("src/a.rs"), "the parent could not, so the child cannot");
    assert!(
        !child.grants.allows(&zorix_core::workspace_grant(Action::Write, "out/x").unwrap()),
        "no write from the parent"
    );
    assert!(!child.grants.allows(&Permission::new(Action::Network, "http:example.com")), "no network from the parent");
    assert!(!child.grants.allows(&Permission::use_tool("probe.write")), "the parent lacked that tool");
}

#[tokio::test]
async fn the_parents_budget_is_shared_among_the_children_and_files_narrow_what_they_read() {
    let w = world().await;
    make(&w.store, "lead", &["agent:researcher", "read:*"], |s| {
        s.limits.max_cost_micros = Some(1_000);
        s.limits.max_tool_calls = 9;
        s.limits.max_iterations = 7;
    })
    .await;
    make(&w.store, "researcher", &["read:*"], |s| s.limits.max_tool_calls = 50).await;
    let done = w.run("lead", "ASK researcher: one | researcher: two").await;
    assert_eq!(done.state, S::Completed, "{:?}", done.failure);
    let kids = &w.tree(done.id).await[1..];
    assert_eq!(kids.len(), 2);
    for child in kids {
        // 1000 µ$ minus the 100 the lead had spent, shared by two.
        assert_eq!(child.limits.max_cost_micros, Some(450), "{:?}", child.limits);
        assert!(child.limits.max_tool_calls <= 4, "{}", child.limits.max_tool_calls);
        assert!(child.limits.max_iterations <= 7);
    }
    // The cost of the children is part of the lead's.
    assert_eq!(done.usage.cost_micros, 100 * 4);

    let new = w.ask("lead", AgentAsk { prompt: "ASK researcher: scoped".into(), ..AgentAsk::default() }).await;
    assert!(new.grants.allows(&Permission::use_tool("agents.delegate")));
}

#[tokio::test]
async fn an_agent_already_in_the_chain_cannot_be_asked_again() {
    let w = world().await;
    // Both are allowed to ask each other, so only the chain guard stands between them and a loop.
    make(&w.store, "lead", &["agent:researcher", "agent:lead"], |_| {}).await;
    make(&w.store, "researcher", &["agent:lead", "agent:researcher"], |_| {}).await;
    // The researcher is told to ask the lead, which would loop.
    let done = w.run("lead", "ASK researcher: ASK lead: round we go").await;
    assert_eq!(done.state, S::Completed, "{:?}", done.failure);
    let all = w.tree(done.id).await;
    assert_eq!(all.len(), 2, "the researcher's own delegation was refused");
    let detail = w.engine.detail(&tenant(), all[1].id).await.unwrap();
    let refused = detail.tool_calls.iter().find(|c| c.tool == "agents.delegate").expect("it tried");
    assert!(refused.error.as_deref().unwrap_or("").contains("would loop"), "{refused:?}");
}

#[tokio::test]
async fn the_parents_list_of_models_holds_for_every_model_a_child_might_use() {
    let w = world().await;
    make(&w.store, "lead", &["agent:writer"], |s| s.models.allowed = vec!["demo".into()]).await;
    make(&w.store, "writer", &[], |s| s.models.default = "anthropic/claude-test".into()).await;
    let done = w.run("lead", "ASK writer: draft").await;
    assert_eq!(done.state, S::Completed, "{:?}", done.failure);
    let text = done.result.unwrap().text;
    assert!(text.contains("is not allowed to use") && text.contains("anthropic/claude-test"), "{text}");
    assert_eq!(w.tree(done.id).await.len(), 1);
}

#[tokio::test]
async fn the_available_agents_are_listed_for_access_and_for_a_team_and_a_team_is_recorded() {
    let w = world().await;
    make(&w.store, "lead", &["agent:researcher"], |_| {}).await;
    make(&w.store, "researcher", &[], |_| {}).await;
    make(&w.store, "writer", &[], |_| {}).await;
    let own = w.ask("lead", AgentAsk { prompt: "hi".into(), ..AgentAsk::default() }).await;
    let text = own.request.instructions.clone().unwrap();
    assert!(text.contains("<available_agents>") && text.contains("- researcher: RESEARCHER."), "{text}");
    assert!(!text.contains("- writer:"), "only whom it may ask");
    assert!(text.contains("up to 8 tasks") && text.contains("3 levels"));
    assert!(own.agent.as_ref().unwrap().team.is_none());

    let team = w
        .ask(
            "lead",
            AgentAsk {
                prompt: "hi".into(),
                team: Some("desk".into()),
                extra_access: vec!["agent:writer".into()],
                ..AgentAsk::default()
            },
        )
        .await;
    let text = team.request.instructions.clone().unwrap();
    assert!(text.contains("- researcher:") && text.contains("- writer:"), "{text}");
    assert_eq!(team.agent.as_ref().unwrap().team.as_deref(), Some("desk"));
    assert!(team.grants.allows(&Permission::new(Action::Execute, "agent:writer")));

    // `agent:*` lists everyone but the agent itself.
    make(&w.store, "boss", &["agent:*"], |_| {}).await;
    let boss = w.ask("boss", AgentAsk { prompt: "hi".into(), ..AgentAsk::default() }).await;
    let text = boss.request.instructions.unwrap();
    assert!(text.contains("- lead:") && text.contains("- researcher:") && text.contains("- writer:"));
    assert!(!text.contains("- boss:"));
}

#[tokio::test]
async fn an_expected_output_and_the_handoff_reach_the_child() {
    let w = world().await;
    make(&w.store, "lead", &["agent:researcher"], |_| {}).await;
    make(&w.store, "researcher", &[], |_| {}).await;
    // The model of this test does not send these fields, so the planner is asked directly.
    use zorix_core::{DelegateRequest, DelegationPlanner, PlanContext};
    let lead = w.run("lead", "ASK researcher: warm up").await;
    let planner = AgentPlanner::new(w.store.clone(), Arc::new(Anything));
    let request = DelegateRequest::parse(&json!({"tasks": [{
        "agent": "researcher", "task": "list three", "input": {"topic": "tea"},
        "expected_output": {"type": "object", "required": ["items"]}, "context": ["be brief"]
    }]}))
    .unwrap();
    let planned = planner
        .plan(PlanContext { parent: &lead, call_id: "call_1_0", ancestors: vec![], descendants: 0 }, &request)
        .await;
    // The lead's run is finished, so its budget is spent only if it had a limit: it has none here.
    let planned = planned.unwrap();
    let child = &planned[0].new;
    assert_eq!(child.request.prompt, "list three");
    assert_eq!(child.request.expected_output, Some(json!({"type": "object", "required": ["items"]})));
    assert_eq!(child.request.input, Some(json!({"input": {"topic": "tea"}, "notes": ["be brief"]})));
    assert_eq!((child.parent, child.depth), (Some(lead.id), 1));
}

#[tokio::test]
async fn a_child_runs_at_the_level_of_the_run_that_asked_it() {
    use zorix_core::{Autonomy, AutonomyLevel, DelegateRequest, DelegationPlanner, PlanContext};
    let w = world().await;
    make(&w.store, "lead", &["agent:researcher"], |_| {}).await;
    make(&w.store, "researcher", &[], |_| {}).await;
    let own = AgentAsk {
        prompt: "ASK researcher: warm up".into(),
        autonomy: Some(Autonomy { level: AutonomyLevel::Ask, still_ask: vec!["db:ops:write".into()] }),
        ..AgentAsk::default()
    };
    let lead = w.ask("lead", own).await;
    assert_eq!(lead.agent.as_ref().unwrap().autonomy.as_ref().unwrap().level, AutonomyLevel::Ask);
    let created = w.engine.create(lead).await.unwrap();
    let planner = AgentPlanner::new(w.store.clone(), Arc::new(Anything));
    let request = DelegateRequest::parse(&json!({"tasks": [{"agent": "researcher", "task": "list three"}]})).unwrap();
    let planned = planner
        .plan(PlanContext { parent: &created, call_id: "call_1_0", ancestors: vec![], descendants: 0 }, &request)
        .await
        .unwrap();
    let child = planned[0].new.agent.as_ref().unwrap().autonomy.clone().expect("the child has a level");
    assert_eq!(child.level, AutonomyLevel::Ask, "never freer than the run that asked");
    assert_eq!(child.still_ask, vec!["db:ops:write".to_string()]);
}

#[tokio::test]
async fn an_evaluation_refuses_approvals_all_the_way_down_and_can_check_who_was_asked() {
    use zorix_agents::evals::{EvalOptions, Evaluator};
    let w = world().await;
    make(&w.store, "lead", &["agent:researcher", "write:*"], |_| {}).await;
    make(&w.store, "researcher", &["write:*"], |_| {}).await;
    let (agent, version) = {
        let agent = w.store.agent_by_slug(&tenant(), &AgentSlug::new("lead").unwrap()).await.unwrap().unwrap();
        let version = w.store.version(&tenant(), agent.id, agent.current_version).await.unwrap().unwrap();
        (agent, version)
    };
    let case = |name: &str, prompt: &str, approve: &[&str]| zorix_core::StoredCase {
        id: zorix_core::EvalCaseId::new(),
        agent: agent.id,
        case: EvalCase {
            name: name.into(),
            prompt: prompt.into(),
            input: None,
            tags: vec![],
            checks: vec![
                EvalCheck::Delegated { agent: "researcher".into(), min: 1 },
                EvalCheck::Contains { text: "combined".into(), ignore_case: false },
            ],
            judge: None,
            approve: approve.iter().map(|a| a.to_string()).collect(),
            allow_writes: true,
        },
        updated_at: Timestamp(1),
    };
    let evaluator = Evaluator {
        engine: Arc::new(w.engine.clone()),
        store: w.store.clone(),
        resolver: Arc::new(Anything),
        judge: None,
    };
    let options = EvalOptions { concurrency: 1, model: Some("demo".into()), ..EvalOptions::default() };
    let run = evaluator
        .run(
            &agent,
            &version,
            vec![
                case("refuses", "ASK researcher: WRITE it", &[]),
                case("allows", "ASK researcher: WRITE it", &["probe.write"]),
            ],
            options,
            CancellationToken::new(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(run.status, "finished");
    let (refused, allowed) = (&run.results[0], &run.results[1]);
    assert!(refused.passed, "the sub-run's approval was refused and the lead still got an answer: {refused:?}");
    let tree = w.tree(refused.execution.unwrap()).await;
    let child = &tree[1];
    let detail = w.engine.detail(&tenant(), child.id).await.unwrap();
    assert_eq!(detail.approvals[0].status, zorix_core::ApprovalStatus::Denied);
    assert_eq!(detail.approvals[0].decided_by.as_deref(), Some("evaluation"));
    assert!(child.request.metadata.contains_key(zorix_core::EVAL_LABEL), "a sub-run inherits the evaluation label");

    let tree = w.tree(allowed.execution.unwrap()).await;
    let detail = w.engine.detail(&tenant(), tree[1].id).await.unwrap();
    assert_eq!(
        detail.approvals[0].status,
        zorix_core::ApprovalStatus::Approved,
        "a case may approve a tool below it too"
    );
    assert!(allowed.passed, "{allowed:?}");
}
