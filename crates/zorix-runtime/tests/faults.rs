//! A failure of the storage at any commit leaves a state a restart can continue from, and a tool that cannot be repeated safely is
//! never run a second time without a person saying so.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use common::{fast_config, local};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zorix_core::{
    ApprovalDecision, Error, ExecutionEngine, ExecutionRepository, ExecutionState as S, FinishReason, Grants,
    ModelError, ModelGateway, ModelRequest, ModelResponse, NewExecution, Permission, Role, TaskRequest, Tool,
    ToolAnnotations, ToolCallId, ToolCallRequest, ToolContext, ToolError, ToolRegistry, ToolSpec, Usage,
};
use zorix_runtime::testing::{Fault, FaultyRepository, Method};
use zorix_runtime::{Engine, EngineConfig, InMemoryToolRegistry, MemoryRepository};

/// A tool that changes something and cannot be repeated safely: not read-only, not idempotent. It counts how often it really ran.
struct Once {
    spec: ToolSpec,
    runs: AtomicU32,
}

impl Once {
    fn new() -> Arc<Self> {
        let mut spec = ToolSpec::new("ledger.add", "Add", "adds an entry", json!({"type": "object"}));
        spec.annotations =
            ToolAnnotations { read_only: false, destructive: false, idempotent: false, open_world: false };
        Arc::new(Self { spec, runs: AtomicU32::new(0) })
    }

    fn count(&self) -> u32 {
        self.runs.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Tool for Once {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, _ctx: &ToolContext, _args: Value) -> Result<Value, ToolError> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"added": true}))
    }
}

/// Asks for the tool until it has a result, then answers: it does not need a script that knows where a restart lands.
struct Adaptive;

#[async_trait]
impl ModelGateway for Adaptive {
    async fn generate(&self, request: ModelRequest, _cancel: CancellationToken) -> Result<ModelResponse, ModelError> {
        let has_result = request.messages.iter().any(|m| m.role == Role::Tool);
        let (text, calls, finish) = if has_result {
            ("The entry was added.".to_string(), Vec::new(), FinishReason::Stop)
        } else {
            let call = ToolCallRequest {
                id: ToolCallId(String::new()),
                provider_id: Some("p".into()),
                tool: "ledger.add".into(),
                arguments: json!({}),
            };
            (String::new(), vec![call], FinishReason::ToolCalls)
        };
        Ok(ModelResponse::new(request.model, text, calls, Usage::default(), finish))
    }
}

fn engine(repository: Arc<dyn ExecutionRepository>, tool: &Arc<Once>) -> Engine {
    let tools = Arc::new(InMemoryToolRegistry::new());
    tools.register(tool.clone()).unwrap();
    let config = EngineConfig { sole_owner: true, ..fast_config() };
    Engine::builder(repository, Arc::new(Adaptive)).tools(tools).config(config).build()
}

fn request() -> NewExecution {
    NewExecution {
        request: TaskRequest { prompt: "add an entry".into(), model: "sim".into(), ..TaskRequest::default() },
        grants: Grants::none().with(Permission::use_tool("ledger.add")),
        ..NewExecution::default()
    }
}

/// Let the first engine run until it is done or has stopped making progress, then drop it as a crash would.
async fn let_it_run(engine: &Engine, id: zorix_core::ExecutionId) {
    let _ = tokio::time::timeout(Duration::from_millis(250), engine.settle(&local(), id)).await;
    engine.abort();
}

#[tokio::test]
async fn a_storage_failure_at_any_commit_leaves_a_state_a_restart_continues_from_and_the_tool_never_runs_twice_unasked()
{
    let mut reached_the_end = 0;
    let mut asked_a_person = 0;
    for method in [Method::Commit, Method::Create] {
        for fault in [
            Fault::Before(Error::Storage("The disk is full.".into())),
            Fault::AfterWrite(Error::Storage("The connection dropped.".into())),
        ] {
            for at in 1..=14 {
                let inner = Arc::new(MemoryRepository::new());
                let faulty = Arc::new(FaultyRepository::new(inner.clone()));
                faulty.fail(method, at, fault.clone());
                let tool = Once::new();

                // The first program meets the failure (or finishes before it, for a late `at`).
                let first = engine(faulty.clone(), &tool);
                let id = match first.submit(request()).await {
                    Ok(started) => started.id,
                    Err(_) => {
                        // Submitting itself failed (the create or its first commit): the caller was told, and nothing ran.
                        assert_eq!(tool.count(), 0, "{method:?} #{at} {fault:?}");
                        first.abort();
                        continue;
                    }
                };
                let_it_run(&first, id).await;
                drop(first);

                // A new program, as after a restart, over what is stored.
                let second = engine(inner.clone(), &tool);
                let report = second.recover().await.unwrap();
                let mut state = tokio::time::timeout(Duration::from_secs(10), second.settle(&local(), id))
                    .await
                    .unwrap_or_else(|_| panic!("{method:?} #{at} {fault:?}: the run never settled after recovery"))
                    .unwrap();
                let mut asked = report.needs_decision.contains(&id);
                // A call that may have run and cannot be repeated asks a person; the person says yes.
                while state.state == S::WaitingForApproval {
                    asked = true;
                    assert!(tool.count() <= 1, "the tool did not run again before a person was asked");
                    let approval = second
                        .detail(&local(), id)
                        .await
                        .unwrap()
                        .approvals
                        .into_iter()
                        .find(|a| a.status == zorix_core::ApprovalStatus::Pending)
                        .unwrap();
                    let decision = ApprovalDecision {
                        approve: true,
                        decided_by: "tester".into(),
                        note: None,
                        call_hash: Some(approval.call_hash.clone()),
                        automatic: false,
                    };
                    let before = tool.count();
                    second.decide(&local(), id, approval.id, decision).await.unwrap();
                    assert!(tool.count() <= before + 1, "approving lets it run at most once more");
                    state = tokio::time::timeout(Duration::from_secs(10), second.settle(&local(), id))
                        .await
                        .unwrap()
                        .unwrap();
                }

                let label = format!("{method:?} #{at} {fault:?}");
                assert_eq!(state.state, S::Completed, "{label}: {:?}", state.failure);
                assert!(
                    tool.count() == 1 || (tool.count() == 2 && asked),
                    "{label}: the tool ran {} times and nobody was asked",
                    tool.count()
                );
                // The history is whole: events are numbered in order and there is exactly one terminal event.
                let events = inner.events(id, 0, 1_000).await.unwrap();
                assert!(events.windows(2).all(|w| w[0].seq < w[1].seq), "{label}");
                assert_eq!(events.iter().filter(|e| e.kind.name() == "execution.completed").count(), 1, "{label}");
                if faulty.fired() {
                    reached_the_end += 1;
                }
                if asked {
                    asked_a_person += 1;
                }
                second.abort();
            }
        }
    }
    assert!(asked_a_person >= 1, "at least one failure landed where the tool may have run, and a person was asked");
    assert!(reached_the_end >= 10, "the faults really fired in many places: {reached_the_end}");
}
