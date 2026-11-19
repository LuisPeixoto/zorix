//! The AgentRunner: what a run built from an agent can and cannot do.

use zorix_agents::{AgentAsk, ModelResolver, RunnerError, prepare};
use zorix_core::{
    Action, Agent, AgentSlug, AgentSpec, AgentStore, AgentVersion, Grants, Limits, MemoryOptions, Permission,
    RecallMode, SkillFile, SkillPin, SkillRecord, TenantId, Timestamp,
};
use zorix_store::SqliteAgentStore;

/// Resolves a few names the way a router would: a model, an alias with one candidate, and a route with two.
struct Names;

impl ModelResolver for Names {
    fn candidates(&self, model: &str) -> Result<Vec<(Option<String>, String)>, String> {
        let one = |p: &str, m: &str| (Some(p.to_string()), m.to_string());
        match model {
            "demo" => Ok(vec![(Some("sim".into()), "demo".into())]),
            "anthropic/claude-opus-5-5" => Ok(vec![one("anthropic", "claude-opus-5-5")]),
            "openai/gpt-5" => Ok(vec![one("openai", "gpt-5")]),
            "route:mixed" => Ok(vec![one("openai", "gpt-5"), one("anthropic", "claude-opus-5-5")]),
            other => Err(format!("“{other}” is not a model, an alias or a route.")),
        }
    }
}

const NOW: Timestamp = Timestamp(1);

async fn made(store: &SqliteAgentStore, mut spec: AgentSpec) -> (Agent, AgentVersion) {
    spec.slug = AgentSlug::new("scout").unwrap();
    let (agent, version) = store.create_agent(&TenantId::local(), spec, None, NOW).await.unwrap();
    (agent, version)
}

fn spec() -> AgentSpec {
    let mut spec = AgentSpec::new(AgentSlug::new("scout").unwrap(), "Scout", "demo");
    spec.instructions = "Find things out.".into();
    spec
}

fn ask(prompt: &str) -> AgentAsk {
    AgentAsk { prompt: prompt.into(), ..AgentAsk::default() }
}

fn refused(error: RunnerError) -> String {
    match error {
        RunnerError::Refused(text) | RunnerError::Store(text) => text,
    }
}

#[tokio::test]
async fn a_run_records_the_version_and_uses_the_default_model_and_the_agents_own_text() {
    let store = SqliteAgentStore::open_in_memory().unwrap();
    let mut s = spec();
    s.metadata.insert("team".into(), "research".into());
    let (agent, version) = made(&store, s).await;
    let mut request = ask("Look into it");
    request.metadata.insert("screen".into(), "chat".into());
    let new = prepare(&agent, &version, request, &store, &Names).await.unwrap();
    new.validate().unwrap();
    assert_eq!(new.request.model, "demo");
    assert_eq!(new.request.instructions.as_deref(), Some("Find things out."));
    assert_eq!(new.request.prompt, "Look into it");
    let binding = new.agent.as_ref().unwrap();
    assert_eq!((binding.version, binding.hash.as_str()), (1, version.hash.as_str()));
    assert_eq!(binding.allowed_models, vec!["*".to_string()]);
    assert_eq!(new.request.metadata["agent"], "scout");
    assert_eq!(new.request.metadata["agent_version"], "1");
    assert_eq!(new.request.metadata["team"], "research");
    assert_eq!(new.request.metadata["screen"], "chat");
}

#[tokio::test]
async fn only_an_allowed_model_is_accepted_and_the_refusal_names_what_is_allowed() {
    let store = SqliteAgentStore::open_in_memory().unwrap();
    let mut s = spec();
    s.models.default = "anthropic/claude-opus-5-5".into();
    s.models.allowed = vec!["anthropic/*".into()];
    let (agent, version) = made(&store, s).await;

    let ok = prepare(&agent, &version, ask("x"), &store, &Names).await.unwrap();
    assert_eq!(ok.request.model, "anthropic/claude-opus-5-5");

    let mut other = ask("x");
    other.model = Some("openai/gpt-5".into());
    let error = refused(prepare(&agent, &version, other, &store, &Names).await.unwrap_err());
    assert!(error.contains("scout may not use openai/gpt-5") && error.contains("anthropic/*"), "{error}");

    // A route is fine when at least one of its candidates is allowed; the router drops the others when it calls.
    let mut route = ask("x");
    route.model = Some("route:mixed".into());
    let new = prepare(&agent, &version, route, &store, &Names).await.unwrap();
    assert_eq!(new.request.model, "route:mixed");
    assert_eq!(new.agent.unwrap().allowed_models, vec!["anthropic/*".to_string()]);

    // An unknown name is refused with the resolver's own words.
    let mut unknown = ask("x");
    unknown.model = Some("nonsense".into());
    assert!(refused(prepare(&agent, &version, unknown, &store, &Names).await.unwrap_err()).contains("not a model"));
}

#[tokio::test]
async fn skills_are_listed_for_the_model_and_the_skill_tools_follow_what_the_agent_may_do() {
    let store = SqliteAgentStore::open_in_memory().unwrap();
    let hash = "e".repeat(64);
    let record = SkillRecord {
        name: "pdf-tools".into(),
        hash: hash.clone(),
        description: "Reads PDFs. Use when a task mentions PDFs.".into(),
        files: vec![SkillFile { path: "SKILL.md".into(), size: 5, sha256: hash.clone() }],
        license: None,
        compatibility: None,
        metadata: Default::default(),
        allowed_tools: vec![],
        source: None,
        imported_at: NOW,
    };
    store.record_skill(&TenantId::local(), record).await.unwrap();

    let mut s = spec();
    s.skills = vec![SkillPin { name: "pdf-tools".into(), hash: hash.clone() }];
    let (agent, version) = made(&store, s.clone()).await;
    let new = prepare(&agent, &version, ask("read it"), &store, &Names).await.unwrap();
    let instructions = new.request.instructions.unwrap();
    assert!(
        instructions.contains("<available_skills>\n- pdf-tools: Reads PDFs. Use when a task mentions PDFs.\n"),
        "{instructions}"
    );
    assert!(
        new.grants.allows(&Permission::use_tool("skill.load"))
            && new.grants.allows(&Permission::use_tool("skill.read"))
    );
    assert!(!new.grants.allows(&Permission::use_tool("skill.run")), "running a script needs code access");

    // With code access, the skill's scripts may run (in the sandbox).
    let store2 = SqliteAgentStore::open_in_memory().unwrap();
    store2
        .record_skill(&TenantId::local(), store.skill(&TenantId::local(), "pdf-tools").await.unwrap().unwrap())
        .await
        .unwrap();
    s.slug = AgentSlug::new("coder").unwrap();
    s.access = vec!["code:python".into()];
    let (agent, version) = store2.create_agent(&TenantId::local(), s, None, NOW).await.unwrap();
    let new = prepare(&agent, &version, ask("run"), &store2, &Names).await.unwrap();
    assert!(new.grants.allows(&Permission::use_tool("skill.run")));
    assert!(new.grants.allows(&Permission::new(Action::Execute, "code:python")));

    // A skill that is missing from the library stops the run with a way to fix it.
    let empty = SqliteAgentStore::open_in_memory().unwrap();
    let mut lost = spec();
    lost.skills = vec![SkillPin { name: "gone".into(), hash: "f".repeat(64) }];
    let (agent, version) = empty.create_agent(&TenantId::local(), lost, None, NOW).await.unwrap();
    let error = refused(prepare(&agent, &version, ask("x"), &empty, &Names).await.unwrap_err());
    assert!(error.contains("gone") && error.contains("Import it again"), "{error}");
}

#[tokio::test]
async fn a_request_can_only_narrow_what_the_agent_may_do() {
    let store = SqliteAgentStore::open_in_memory().unwrap();
    let mut s = spec();
    s.tools = vec!["math.*".into(), "clock.now".into()];
    s.access = vec!["repo:acme/api:write".into(), "http:api.example.com".into()];
    s.limits = Limits { max_iterations: 10, max_total_tokens: Some(50_000), ..Limits::default() };
    s.memory =
        MemoryOptions { conversation: true, long_term: true, knowledge: RecallMode::Auto, operations: RecallMode::Off };
    let (agent, version) = made(&store, s).await;

    // No narrowing: the agent's own grants, limits and memory.
    let full = prepare(&agent, &version, ask("x"), &store, &Names).await.unwrap();
    assert!(full.grants.allows(&Permission::new(Action::Write, "github:acme/api")));
    assert_eq!(full.limits.max_iterations, 10);

    // A request that asks for read-only on the repository, one tool, and things the agent never had.
    let mut narrow = ask("x");
    narrow.narrow = Some(
        Grants::none()
            .with(Permission::new(Action::Read, "github:acme/api"))
            .with(Permission::use_tool("math.evaluate"))
            .with(Permission::new(Action::Write, "workspace:*"))
            .with(Permission::new(Action::Network, "http:evil.example.com")),
    );
    narrow.limits = Some(Limits { max_iterations: 99, max_total_tokens: None, max_tool_calls: 3, ..Limits::default() });
    narrow.memory = Some(MemoryOptions::none());
    let run = prepare(&agent, &version, narrow, &store, &Names).await.unwrap();
    assert!(run.grants.allows(&Permission::new(Action::Read, "github:acme/api")));
    assert!(!run.grants.allows(&Permission::new(Action::Write, "github:acme/api")), "the request dropped write");
    assert!(run.grants.allows(&Permission::use_tool("math.evaluate")));
    assert!(!run.grants.allows(&Permission::use_tool("clock.now")), "and the other tool");
    assert!(
        !run.grants.allows(&Permission::new(Action::Write, "workspace:notes.txt")),
        "the agent never had workspace access"
    );
    assert!(!run.grants.allows(&Permission::new(Action::Network, "http:evil.example.com")), "nor that host");
    assert!(
        !run.grants.allows(&Permission::new(Action::Network, "http:api.example.com")),
        "the request did not name this one"
    );
    assert_eq!(run.limits.max_iterations, 10, "asking for more does not raise the agent's cap");
    assert_eq!(run.limits.max_total_tokens, Some(50_000), "asking for none does not remove it");
    assert_eq!(run.limits.max_tool_calls, 3);
    assert_eq!(run.memory, MemoryOptions::none());
}
