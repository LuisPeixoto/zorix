//! Shared by the engine tests: a harness with a scripted model and a few small tools.
#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::sync::Notify;
use zorix_core::{
    Execution, ExecutionEngine, ExecutionId, Grants, Limits, NewExecution, Permission, TaskRequest, TenantId, Tool,
    ToolAnnotations, ToolContext, ToolError, ToolRegistry, ToolSpec,
};
use zorix_runtime::{Engine, EngineConfig, InMemoryToolRegistry, MemoryRepository, RetryPolicy};
use zorix_sim::{ScriptedModel, Turn};

/// A tool that counts how many times it really ran.
pub struct Counting {
    spec: ToolSpec,
    pub runs: Arc<AtomicU32>,
    behaviour: Behaviour,
    pub started: Arc<Notify>,
}

pub enum Behaviour {
    Echo,
    Write,
    /// Waits this long, or until cancelled.
    Slow(Duration),
    /// Fails with a retryable error this many times, then works.
    Flaky(u32),
    Fails,
    /// Like `Slow`, but declared safe to repeat.
    SlowRead(Duration),
}

impl Counting {
    pub fn new(id: &str, behaviour: Behaviour) -> Arc<Self> {
        let schema = match behaviour {
            Behaviour::Write => {
                json!({"type": "object", "properties": {"path": {"type": "string"}, "content": {"type": "string"}}, "required": ["path", "content"], "additionalProperties": false})
            }
            _ => json!({"type": "object", "properties": {"text": {"type": "string"}}, "additionalProperties": false}),
        };
        let mut spec = ToolSpec::new(id, id, format!("The {id} test tool"), schema);
        spec.timeout_ms = 5_000;
        spec.annotations = match behaviour {
            Behaviour::Echo | Behaviour::Flaky(_) | Behaviour::SlowRead(_) => ToolAnnotations::safe_read(),
            Behaviour::Write => {
                ToolAnnotations { read_only: false, destructive: true, idempotent: false, open_world: false }
            }
            Behaviour::Slow(_) | Behaviour::Fails => {
                ToolAnnotations { read_only: false, destructive: false, idempotent: false, open_world: false }
            }
        };
        Arc::new(Self { spec, runs: Arc::new(AtomicU32::new(0)), behaviour, started: Arc::new(Notify::new()) })
    }

    pub fn count(&self) -> u32 {
        self.runs.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Tool for Counting {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let attempt = self.runs.fetch_add(1, Ordering::SeqCst) + 1;
        self.started.notify_waiters();
        match &self.behaviour {
            Behaviour::Echo => Ok(json!({ "echo": args.get("text").cloned().unwrap_or(Value::Null) })),
            Behaviour::Write => Ok(json!({ "written": args["path"] })),
            Behaviour::Slow(time) | Behaviour::SlowRead(time) => {
                tokio::select! {
                    _ = tokio::time::sleep(*time) => Ok(json!({"slept": true})),
                    _ = ctx.cancel.cancelled() => Err(ToolError::Failed("stopped".into())),
                }
            }
            Behaviour::Flaky(times) if attempt <= *times => Err(ToolError::Retryable("try again".into())),
            Behaviour::Flaky(_) => Ok(json!({"finally": true})),
            Behaviour::Fails => Err(ToolError::Failed("the disk is full".into())),
        }
    }
}

pub struct Harness {
    pub engine: Engine,
    pub repo: Arc<MemoryRepository>,
    pub model: Arc<ScriptedModel>,
    pub echo: Arc<Counting>,
    pub write: Arc<Counting>,
    pub slow: Arc<Counting>,
    pub flaky: Arc<Counting>,
    pub fails: Arc<Counting>,
}

pub fn fast_config() -> EngineConfig {
    EngineConfig {
        workers: 2,
        lease_ttl_ms: 1_000,
        retry: RetryPolicy { base_delay_ms: 1, max_delay_ms: 5 },
        ..EngineConfig::default()
    }
}

pub fn registry() -> (Arc<InMemoryToolRegistry>, [Arc<Counting>; 5]) {
    let tools = [
        Counting::new("echo", Behaviour::Echo),
        Counting::new("files.write", Behaviour::Write),
        Counting::new("slow", Behaviour::Slow(Duration::from_secs(30))),
        Counting::new("flaky", Behaviour::Flaky(2)),
        Counting::new("fails", Behaviour::Fails),
    ];
    let registry = Arc::new(InMemoryToolRegistry::new());
    for tool in &tools {
        registry.register(tool.clone()).unwrap();
    }
    (registry, tools)
}

pub fn harness(turns: Vec<Turn>) -> Harness {
    let repo = Arc::new(MemoryRepository::new());
    let model = Arc::new(ScriptedModel::new(turns));
    let (tools, [echo, write, slow, flaky, fails]) = registry();
    let engine = Engine::builder(repo.clone(), model.clone()).tools(tools).config(fast_config()).build();
    Harness { engine, repo, model, echo, write, slow, flaky, fails }
}

/// A request that may use exactly these tools.
pub fn request(prompt: &str, tools: &[&str]) -> NewExecution {
    NewExecution {
        request: TaskRequest { prompt: prompt.into(), model: "sim".into(), ..TaskRequest::default() },
        grants: grants_for(tools),
        ..NewExecution::default()
    }
}

pub fn grants_for(tools: &[&str]) -> Grants {
    tools.iter().fold(Grants::none(), |grants, tool| grants.with(Permission::use_tool(tool)))
}

pub fn with_limits(mut new: NewExecution, limits: Limits) -> NewExecution {
    new.limits = limits;
    new
}

pub fn local() -> TenantId {
    TenantId::local()
}

/// Wait for the execution to stop moving, but never forever.
pub async fn settled(engine: &Engine, id: ExecutionId) -> Execution {
    tokio::time::timeout(Duration::from_secs(10), engine.settle(&local(), id))
        .await
        .expect("the execution settles within 10 seconds")
        .expect("settle")
}

/// Create, start and wait.
pub async fn run(engine: &Engine, new: NewExecution) -> Execution {
    let started = engine.submit(new).await.expect("submit");
    settled(engine, started.id).await
}

pub async fn kinds(engine: &Engine, id: ExecutionId) -> Vec<&'static str> {
    engine.events(&local(), id, 0).await.unwrap().iter().map(|e| e.kind.name()).collect()
}

/// True when `wanted` appears in `all` in this order (other items may sit between them).
pub fn in_order(all: &[&str], wanted: &[&str]) -> bool {
    let mut at = 0;
    for name in all {
        if at < wanted.len() && *name == wanted[at] {
            at += 1;
        }
    }
    at == wanted.len()
}

/// Poll until the execution is in `state`.
pub async fn wait_for_state(engine: &Engine, id: ExecutionId, state: zorix_core::ExecutionState) -> Execution {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let current = engine.get(&local(), id).await.unwrap();
            if current.state == state {
                return current;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the execution never reached {state}"))
}

/// The one approval that is waiting for an answer.
pub async fn pending_approval(engine: &Engine, id: ExecutionId) -> zorix_core::Approval {
    let detail = engine.detail(&local(), id).await.unwrap();
    detail.approvals.into_iter().find(|a| a.status == zorix_core::ApprovalStatus::Pending).expect("a pending approval")
}

/// Answer the pending approval.
pub async fn answer(engine: &Engine, id: ExecutionId, approve: bool, note: Option<&str>) -> Execution {
    let approval = pending_approval(engine, id).await;
    let decision = zorix_core::ApprovalDecision {
        approve,
        decided_by: "tester".into(),
        note: note.map(str::to_string),
        call_hash: Some(approval.call_hash.clone()),
        automatic: false,
    };
    engine.decide(&local(), id, approval.id, decision).await.expect("decide")
}
