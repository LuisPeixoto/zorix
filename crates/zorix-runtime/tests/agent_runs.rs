//! A run made by an agent records the version it came from, hands it to its tools, and cannot use a model the agent may not
//! use, even through a gateway that is not the router (ADR-0024, ADR-0027).

mod common;

use std::sync::Arc;

use async_trait::async_trait;
use common::*;
use serde_json::{Value, json};
use zorix_core::{
    Agent, AgentBinding, AgentId, AgentSlug, AgentSpec, AgentVersion, ExecutionEngine, ExecutionState as S,
    FailureKind, Timestamp, Tool, ToolContext, ToolError, ToolRegistry, ToolSpec,
};
use zorix_runtime::{Engine, InMemoryToolRegistry, MemoryRepository};
use zorix_sim::{ScriptedModel, Turn};

/// Remembers the agent every call was made for.
struct Looks {
    spec: ToolSpec,
    seen: parking_lot::Mutex<Vec<Option<AgentBinding>>>,
}

#[async_trait]
impl Tool for Looks {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, ctx: &ToolContext, _args: Value) -> Result<Value, ToolError> {
        self.seen.lock().push(ctx.agent.clone());
        Ok(json!({"ok": true}))
    }
}

fn binding(allowed: &[&str]) -> AgentBinding {
    let slug = AgentSlug::new("scout").unwrap();
    let mut spec = AgentSpec::new(slug.clone(), "Scout", "demo");
    spec.models.allowed = allowed.iter().map(|s| s.to_string()).collect();
    let agent = Agent {
        id: AgentId::new(),
        tenant: local(),
        slug,
        current_version: 3,
        archived: false,
        created_at: Timestamp(1),
        updated_at: Timestamp(1),
    };
    let version =
        AgentVersion { agent: agent.id, version: 3, hash: spec.hash(), spec, note: None, created_at: Timestamp(1) };
    AgentBinding::of(&agent, &version)
}

fn engine(registry: Arc<InMemoryToolRegistry>, model: Arc<ScriptedModel>) -> Engine {
    Engine::builder(Arc::new(MemoryRepository::new()), model).tools(registry).config(fast_config()).build()
}

#[tokio::test]
async fn the_version_is_recorded_on_the_run_and_given_to_every_tool_call() {
    let registry = Arc::new(InMemoryToolRegistry::new());
    let mut spec = ToolSpec::new("agent.look", "look", "Looks at who is asking.", json!({"type": "object"}));
    spec.annotations = zorix_core::ToolAnnotations::safe_read();
    let looks = Arc::new(Looks { spec, seen: Default::default() });
    registry.register(looks.clone()).unwrap();
    let engine =
        engine(registry, Arc::new(ScriptedModel::new([Turn::call("agent.look", json!({})), Turn::Say("done".into())])));

    let mut new = request("look", &["agent.look"]);
    new.agent = Some(binding(&["*"]));
    let want = new.agent.clone();
    let done = run(&engine, new).await;
    assert_eq!(done.state, S::Completed);
    assert_eq!(done.agent, want, "the execution records the binding, version and hash included");
    assert_eq!(done.agent.as_ref().unwrap().version, 3);
    assert_eq!(looks.seen.lock().as_slice(), &[want], "and the tool saw the same agent");

    // A run that no agent made has none.
    let plain = {
        let registry = Arc::new(InMemoryToolRegistry::new());
        registry.register(looks.clone()).unwrap();
        let engine = self::engine(
            registry,
            Arc::new(ScriptedModel::new([Turn::call("agent.look", json!({})), Turn::Say("x".into())])),
        );
        run(&engine, request("look", &["agent.look"])).await
    };
    assert!(plain.agent.is_none());
    assert_eq!(looks.seen.lock().last().unwrap(), &None);
}

#[tokio::test]
async fn a_model_the_agent_may_not_use_fails_the_run_with_a_clear_reason() {
    let registry = Arc::new(InMemoryToolRegistry::new());
    // The scripted model is a direct gateway, not the router, so this is the runtime's own check.
    let engine = engine(registry, Arc::new(ScriptedModel::new([Turn::Say("hello".into())])));
    let mut new = request("hi", &[]);
    new.agent = Some(binding(&["anthropic/*"]));
    let done = run(&engine, new).await;
    assert_eq!(done.state, S::Failed);
    let failure = done.failure.unwrap();
    assert_eq!(failure.kind, FailureKind::Model);
    assert!(failure.message.contains("may not use it"), "{}", failure.message);
}

#[tokio::test]
async fn an_agent_run_must_name_the_models_it_may_use() {
    let registry = Arc::new(InMemoryToolRegistry::new());
    let engine = engine(registry, Arc::new(ScriptedModel::new([Turn::Say("hello".into())])));
    let mut new = request("hi", &[]);
    let mut bad = binding(&["*"]);
    bad.allowed_models.clear();
    new.agent = Some(bad);
    let error = engine.create(new).await.unwrap_err().to_string();
    assert!(error.contains("must name the models it may use"), "{error}");
}
