//! Delegation (ADR-0029, ADR-0030): a parent suspends without holding a worker, children run, results come back, and the tree
//! obeys one set of rules (cancel, approvals, guards, recovery).

mod common;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use common::*;
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zorix_core::delegation::TOOL;
use zorix_core::{
    DelegateRequest, DelegateTool, DelegationPlanner, ExecutionEngine, ExecutionRepository, ExecutionState as S,
    Grants, ListFilter, MAX_DEPTH, ModelError, ModelGateway, ModelRequest, ModelResponse, NewExecution, PlanContext,
    PlannedChild, TaskRequest, ToolCallRequest, Usage,
};
use zorix_core::{FinishReason, Role, ToolCallId};
use zorix_runtime::{Engine, InMemoryToolRegistry, MemoryRepository};

/// Plans one child per task, with the parent's own grants (so a child may use the same tools), unless told to refuse.
struct Planner {
    refuse: Mutex<Option<String>>,
}

impl Planner {
    fn new() -> Arc<Self> {
        Arc::new(Self { refuse: Mutex::new(None) })
    }
}

#[async_trait]
impl DelegationPlanner for Planner {
    async fn plan(&self, context: PlanContext<'_>, request: &DelegateRequest) -> Result<Vec<PlannedChild>, String> {
        if let Some(why) = self.refuse.lock().clone() {
            return Err(why);
        }
        Ok(request
            .tasks
            .iter()
            .map(|task| PlannedChild {
                task: task.clone(),
                new: NewExecution {
                    request: TaskRequest { prompt: task.task.clone(), model: "sim".into(), ..TaskRequest::default() },
                    grants: context.parent.grants.clone(),
                    limits: context.parent.limits,
                    ..NewExecution::default()
                },
            })
            .collect())
    }
}

/// A model that decides by what it is asked, so parallel children cannot disturb each other's script.
/// - a prompt starting with `ASK` delegates to the tasks after it, separated by `|`; once it has the results it answers with them;
/// - `RECURSE` always delegates one task `RECURSE`;
/// - `FAIL` fails; `HANG` never answers; `TOOL` calls the `files.write` tool once, then answers;
/// - anything else answers `done: <prompt>`.
struct ByPrompt;

fn first_user(request: &ModelRequest) -> String {
    request.messages.iter().find(|m| m.role == Role::User).map(|m| m.content.clone()).unwrap_or_default()
}

fn has_tool_result(request: &ModelRequest) -> Option<String> {
    request.messages.iter().rev().find(|m| m.role == Role::Tool).map(|m| m.content.clone())
}

fn say(request: &ModelRequest, text: String) -> ModelResponse {
    let usage = Usage { input_tokens: 10, output_tokens: 5, cost_micros: 100, model_calls: 1, ..Usage::default() };
    ModelResponse::new(request.model.clone(), text, vec![], usage, FinishReason::Stop)
}

fn call(request: &ModelRequest, tool: &str, arguments: Value) -> ModelResponse {
    let usage = Usage { input_tokens: 10, output_tokens: 5, cost_micros: 100, model_calls: 1, ..Usage::default() };
    let call = ToolCallRequest { id: ToolCallId(String::new()), provider_id: None, tool: tool.into(), arguments };
    ModelResponse::new(request.model.clone(), String::new(), vec![call], usage, FinishReason::ToolCalls)
}

fn delegate_args(tasks: &[&str]) -> Value {
    json!({"tasks": tasks.iter().map(|t| json!({"agent": "helper", "task": t})).collect::<Vec<_>>()})
}

#[async_trait]
impl ModelGateway for ByPrompt {
    async fn generate(&self, request: ModelRequest, cancel: CancellationToken) -> Result<ModelResponse, ModelError> {
        let prompt = first_user(&request);
        let results = has_tool_result(&request);
        if prompt.starts_with("HANG") {
            cancel.cancelled().await;
            return Err(ModelError::Cancelled);
        }
        if prompt.starts_with("FAIL") {
            return Err(ModelError::Permanent("the model broke".into()));
        }
        if let Some(list) = prompt.strip_prefix("ASK ") {
            return Ok(match results {
                Some(text) => say(&request, format!("combined: {text}")),
                None => call(&request, TOOL, delegate_args(&list.split('|').map(str::trim).collect::<Vec<_>>())),
            });
        }
        if prompt.starts_with("RECURSE") {
            return Ok(match results {
                Some(text) => say(&request, format!("below: {}", text.chars().take(60).collect::<String>())),
                None => call(&request, TOOL, delegate_args(&["RECURSE"])),
            });
        }
        if prompt.starts_with("TOOL") {
            return Ok(match results {
                Some(_) => say(&request, "wrote it".into()),
                None => call(&request, "files.write", json!({"path": "a.txt", "content": "x"})),
            });
        }
        Ok(say(&request, format!("done: {prompt}")))
    }
}

struct World {
    engine: Engine,
    repo: Arc<MemoryRepository>,
    planner: Arc<Planner>,
}

fn world(workers: usize) -> World {
    let repo = Arc::new(MemoryRepository::new());
    let planner = Planner::new();
    let registry = Arc::new(InMemoryToolRegistry::new());
    zorix_core::ToolRegistry::register(&*registry, Arc::new(DelegateTool::new())).unwrap();
    let (_, [_, write, ..]) = registry_with_write();
    zorix_core::ToolRegistry::register(&*registry, write).unwrap();
    let mut config = fast_config();
    config.workers = workers;
    let engine = Engine::builder(repo.clone(), Arc::new(ByPrompt))
        .tools(registry)
        .delegation(planner.clone())
        .config(config)
        .build();
    World { engine, repo, planner }
}

fn registry_with_write() -> (Arc<InMemoryToolRegistry>, [Arc<Counting>; 5]) {
    registry()
}

fn root(prompt: &str) -> NewExecution {
    let mut new = request(prompt, &["agents.delegate", "files.write"]);
    new.grants = Grants::none()
        .with(zorix_core::Permission::use_tool("agents.delegate"))
        .with(zorix_core::Permission::use_tool("files.write"))
        .with(zorix_core::Permission::new(zorix_core::Action::Write, "workspace:*"));
    new
}

async fn tree(engine: &Engine, id: zorix_core::ExecutionId) -> Vec<zorix_core::Execution> {
    engine.tree(&local(), id).await.unwrap()
}

#[tokio::test]
async fn a_parent_delegates_two_tasks_in_parallel_and_gets_both_results_back() {
    let w = world(2);
    let done = run(&w.engine, root("ASK first thing | second thing")).await;
    assert_eq!(done.state, S::Completed, "{:?}", done.failure);
    let text = done.result.unwrap().text;
    assert!(text.contains("done: first thing") && text.contains("done: second thing"), "{text}");
    assert!(
        text.find("first thing").unwrap() < text.find("second thing").unwrap(),
        "results keep the order of the tasks"
    );

    let all = tree(&w.engine, done.id).await;
    assert_eq!(all.len(), 3);
    for child in &all[1..] {
        assert_eq!((child.parent, child.root, child.depth), (Some(done.id), Some(done.id), 1));
        assert_eq!(child.state, S::Completed);
        assert!(child.conversation.is_none());
    }
    // What the children used is part of what the parent used: three model calls of 100 µ$ each, plus the parent's own two.
    assert_eq!(done.usage.cost_micros, 100 * 4, "the parent's two calls and the children's two");
    assert_eq!(done.usage.model_calls, 4, "the children's model calls are the parent's too");

    let names = kinds(&w.engine, done.id).await;
    assert!(
        in_order(
            &names,
            &[
                "execution.waiting_children",
                "execution.child.settled",
                "execution.child.settled",
                "execution.completed"
            ]
        ),
        "{names:?}"
    );
    assert_eq!(names.iter().filter(|n| **n == "execution.child.started").count(), 2);
    let detail = w.engine.detail(&local(), done.id).await.unwrap();
    let record = detail.tool_calls.iter().find(|c| c.tool == TOOL).unwrap();
    assert_eq!(record.status, zorix_core::ToolCallStatus::Succeeded);
    let results = record.result.as_ref().unwrap()["results"].as_array().unwrap().clone();
    assert_eq!(results.len(), 2);
    assert_eq!((results[0]["state"].as_str(), results[0]["child_id"].as_str().is_some()), (Some("completed"), true));
}

#[tokio::test]
async fn waiting_parents_do_not_hold_a_worker_so_they_cannot_deadlock_a_small_pool() {
    // One worker, one parent: if the parent kept the worker while it waited, its child could never run.
    let w = world(1);
    let done = run(&w.engine, root("ASK only child")).await;
    assert_eq!(done.state, S::Completed, "{:?}", done.failure);

    // Four parents each waiting for a child, two workers.
    let w = world(2);
    let mut ids = Vec::new();
    for i in 0..4 {
        ids.push(w.engine.submit(root(&format!("ASK child {i}"))).await.unwrap().id);
    }
    for id in ids {
        let done = settled(&w.engine, id).await;
        assert_eq!(done.state, S::Completed, "{:?}", done.failure);
    }
}

#[tokio::test]
async fn a_child_that_fails_is_reported_to_its_parent_not_raised() {
    let w = world(2);
    let done = run(&w.engine, root("ASK FAIL now | fine")).await;
    assert_eq!(done.state, S::Completed, "the parent decides what a failure means: {:?}", done.failure);
    let text = done.result.unwrap().text;
    assert!(
        text.contains("\"state\":\"failed\"") && text.contains("the model broke") && text.contains("done: fine"),
        "{text}"
    );
}

#[tokio::test]
async fn cancelling_the_parent_cancels_everything_below_it() {
    let w = world(3);
    let started = w.engine.submit(root("ASK HANG one | HANG two")).await.unwrap();
    wait_for_state(&w.engine, started.id, S::WaitingForChildren).await;
    let kids = loop {
        let kids = tree(&w.engine, started.id).await;
        if kids.len() == 3 && kids[1..].iter().all(|k| k.state == S::Running || k.state == S::WaitingForModel) {
            break kids;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    assert_eq!(kids.len(), 3);
    w.engine.cancel(&local(), started.id, Some("stop".into())).await.unwrap();
    let all = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let all = tree(&w.engine, started.id).await;
            if all.iter().all(|e| e.state.is_terminal()) {
                return all;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("every execution of the tree ends");
    assert!(all.iter().all(|e| e.state == S::Cancelled), "{:?}", all.iter().map(|e| e.state).collect::<Vec<_>>());
    assert_eq!(all[0].cancel_reason.as_deref(), Some("stop"));
    assert!(all[1].cancel_reason.is_some());
}

#[tokio::test]
async fn a_child_cancelled_on_its_own_only_changes_what_the_parent_is_told() {
    let w = world(3);
    let started = w.engine.submit(root("ASK HANG slow | quick")).await.unwrap();
    wait_for_state(&w.engine, started.id, S::WaitingForChildren).await;
    let child = loop {
        let all = tree(&w.engine, started.id).await;
        if let Some(hung) = all.iter().skip(1).find(|e| e.request.prompt.starts_with("HANG"))
            && (hung.state == S::Running || hung.state == S::WaitingForModel)
        {
            break hung.id;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    w.engine.cancel(&local(), child, Some("not needed".into())).await.unwrap();
    let done = settled(&w.engine, started.id).await;
    assert_eq!(done.state, S::Completed, "{:?}", done.failure);
    let text = done.result.unwrap().text;
    assert!(
        text.contains("\"state\":\"cancelled\"") && text.contains("not needed") && text.contains("done: quick"),
        "{text}"
    );
}

#[tokio::test]
async fn a_child_that_needs_a_person_keeps_the_whole_tree_waiting_and_can_be_answered_from_the_root() {
    let w = world(3);
    let started = w.engine.submit(root("ASK TOOL write it")).await.unwrap();
    // The root is waiting for its child, and `settle` says so as soon as something below waits for a person.
    let root_now = settled(&w.engine, started.id).await;
    assert_eq!(root_now.state, S::WaitingForChildren);
    let waiting = loop {
        let pending = w.engine.pending_in_tree(&local(), started.id).await.unwrap();
        if let Some(first) = pending.into_iter().next() {
            break first;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    assert_eq!((waiting.depth, waiting.approval.tool.as_str()), (1, "files.write"));
    assert_ne!(waiting.execution, started.id, "the approval belongs to the child, found from the root");

    let decision = zorix_core::ApprovalDecision {
        approve: true,
        decided_by: "tester".into(),
        note: None,
        call_hash: Some(waiting.approval.call_hash.clone()),
        automatic: false,
    };
    w.engine.decide(&local(), waiting.execution, waiting.approval.id, decision).await.unwrap();
    let done = settled(&w.engine, started.id).await;
    assert_eq!(done.state, S::Completed, "{:?}", done.failure);
    assert!(done.result.unwrap().text.contains("wrote it"));
    assert!(
        done.tool_calls_requested >= 2,
        "the child's tool call counts against the parent's limit: {}",
        done.tool_calls_requested
    );
}

#[tokio::test]
async fn nesting_stops_at_the_depth_limit_with_a_reason_the_model_can_read() {
    let w = world(4);
    let done = run(&w.engine, root("RECURSE")).await;
    assert_eq!(done.state, S::Completed, "{:?}", done.failure);
    let all = tree(&w.engine, done.id).await;
    let deepest = all.iter().map(|e| e.depth).max().unwrap();
    assert_eq!(deepest, MAX_DEPTH, "root plus {MAX_DEPTH} levels, and no more");
    assert_eq!(all.len(), MAX_DEPTH as usize + 1);
    let bottom = all.iter().find(|e| e.depth == MAX_DEPTH).unwrap();
    let detail = w.engine.detail(&local(), bottom.id).await.unwrap();
    let refused = detail.tool_calls.iter().find(|c| c.tool == TOOL).unwrap();
    assert!(refused.error.as_deref().unwrap_or("").contains("deepest level"), "{refused:?}");
}

#[tokio::test]
async fn too_many_descendants_are_refused_before_any_starts() {
    let w = world(2);
    let parent = w.engine.create(root("ASK a | b | c")).await.unwrap();
    // Thirty descendants already exist under this root.
    for i in 0..30 {
        let mut child = request(&format!("old {i}"), &[]);
        child.parent = Some(parent.id);
        child.root = Some(parent.id);
        child.depth = 1;
        w.engine.create(child).await.unwrap();
    }
    w.engine.start(&local(), parent.id).await.unwrap();
    let done = settled(&w.engine, parent.id).await;
    assert_eq!(done.state, S::Completed, "{:?}", done.failure);
    assert!(done.result.unwrap().text.contains("at most 32"), "the refusal reached the model");
    assert_eq!(tree(&w.engine, parent.id).await.len(), 31, "nothing was started");
}

#[tokio::test]
async fn a_planner_that_refuses_gives_the_model_the_reason() {
    let w = world(2);
    *w.planner.refuse.lock() = Some("helper is not on your team".into());
    let done = run(&w.engine, root("ASK anything")).await;
    assert_eq!(done.state, S::Completed, "{:?}", done.failure);
    assert!(done.result.unwrap().text.contains("helper is not on your team"));
    assert_eq!(tree(&w.engine, done.id).await.len(), 1);
}

#[tokio::test]
async fn repeating_a_delegate_call_after_a_crash_finds_the_same_children() {
    let w = world(2);
    let parent = w.engine.create(root("ASK shared")).await.unwrap();
    // The child that an earlier attempt made before it stopped: same key, so the new attempt must not make a second one.
    let mut child = request("shared", &[]);
    child.parent = Some(parent.id);
    child.root = Some(parent.id);
    child.depth = 1;
    child.idempotency_key = Some(zorix_core::child_key(&parent.id.to_string(), "call_1_0", 0));
    let earlier = w.engine.create(child).await.unwrap();
    w.engine.start(&local(), parent.id).await.unwrap();
    let done = settled(&w.engine, parent.id).await;
    assert_eq!(done.state, S::Completed, "{:?}", done.failure);
    let all = tree(&w.engine, parent.id).await;
    assert_eq!(all.len(), 2, "one child, not two");
    assert_eq!(all[1].id, earlier.id);
    assert!(done.result.unwrap().text.contains("done: shared"));
}

#[tokio::test]
async fn recovery_gives_a_waiting_parent_the_results_of_children_that_finished_while_nobody_was_looking() {
    let w = world(3);
    let started = w.engine.submit(root("ASK HANG one")).await.unwrap();
    wait_for_state(&w.engine, started.id, S::WaitingForChildren).await;
    let child = loop {
        let all = tree(&w.engine, started.id).await;
        if all.len() == 2 {
            break all[1].clone();
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    // The child ends behind the engine's back (as if the program had stopped before it could wake its parent).
    let mut ended = w.repo.load(child.id).await.unwrap().unwrap();
    let now = zorix_core::Timestamp(5_000);
    ended.state = S::Failed;
    ended.failure = Some(zorix_core::Failure::new(zorix_core::FailureKind::Internal, "the program stopped"));
    ended.finished_at = Some(now);
    let version = ended.version;
    w.repo.commit(zorix_core::Commit::new(ended, version)).await.unwrap();
    assert_eq!(w.engine.get(&local(), started.id).await.unwrap().state, S::WaitingForChildren, "nothing woke it");

    let report = w.engine.recover().await.unwrap();
    assert!(report.requeued.contains(&started.id), "{report:?}");
    let done = settled(&w.engine, started.id).await;
    assert_eq!(done.state, S::Completed, "{:?}", done.failure);
    assert!(done.result.unwrap().text.contains("the program stopped"));
}

#[tokio::test]
async fn roots_are_listed_without_their_children() {
    let w = world(2);
    let done = run(&w.engine, root("ASK x | y")).await;
    let roots = w.repo.list(&local(), &ListFilter { roots_only: true, ..ListFilter::default() }).await.unwrap();
    assert_eq!(roots.iter().map(|e| e.id).collect::<Vec<_>>(), vec![done.id]);
}
