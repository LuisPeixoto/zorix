//! The memory tools through a real engine: proposals need a person, automatic modes cannot approve them, and what is
//! approved shows up in later executions.

use std::sync::Arc;

use serde_json::json;
use zorix_core::{
    ApprovalDecision, ApprovalStatus, ConversationId, Error, Execution, ExecutionEngine, ExecutionId,
    ExecutionState as S, Grants, MemoryFilter, MemoryId, MemoryOrigin, MemoryStatus, MemoryStore, NewExecution,
    Permission, TaskRequest, TenantId, Timestamp, Visibility,
};
use zorix_memory::MemoryStack;
use zorix_runtime::{Engine, EngineConfig, InMemoryToolRegistry};
use zorix_sim::{ScriptedModel, Turn};
use zorix_store::{SqliteMemoryStore, SqliteRepository};

struct Env {
    _dir: tempfile::TempDir,
    store: Arc<SqliteMemoryStore>,
    engine: Engine,
    model: Arc<ScriptedModel>,
}

fn env(turns: Vec<Turn>) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zorix.sqlite");
    let repository = Arc::new(SqliteRepository::open(&path).unwrap());
    let store = Arc::new(SqliteMemoryStore::open(&path).unwrap());
    let stack = MemoryStack::new(store.clone(), repository.clone(), None);
    let registry = Arc::new(InMemoryToolRegistry::new());
    stack.register_tools(&*registry).unwrap();
    let model = Arc::new(ScriptedModel::new(turns));
    let engine = stack
        .install(Engine::builder(repository, model.clone()).tools(registry))
        .config(EngineConfig::default())
        .build();
    Env { _dir: dir, store, engine, model }
}

fn new_run(prompt: &str, conversation: Option<&str>, workspace: Option<&str>) -> NewExecution {
    NewExecution {
        conversation: conversation.map(|c| ConversationId::new(c).unwrap()),
        request: TaskRequest { prompt: prompt.into(), model: "sim".into(), ..TaskRequest::default() },
        grants: Grants::none()
            .with(Permission::use_tool("memory.remember"))
            .with(Permission::use_tool("memory.search")),
        workspace: workspace.map(Into::into),
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

async fn pending(engine: &Engine, id: ExecutionId) -> zorix_core::Approval {
    let detail = engine.detail(&TenantId::local(), id).await.unwrap();
    detail.approvals.into_iter().find(|a| a.status == ApprovalStatus::Pending).expect("a pending approval")
}

async fn decide(engine: &Engine, id: ExecutionId, approve: bool, automatic: bool) -> Result<Execution, Error> {
    let approval = pending(engine, id).await;
    let decision = ApprovalDecision {
        approve,
        decided_by: if automatic { "cli:auto".into() } else { "luis".into() },
        note: None,
        call_hash: Some(approval.call_hash),
        automatic,
    };
    engine.decide(&TenantId::local(), id, approval.id, decision).await
}

fn remember(content: &str, scope: &str) -> Turn {
    Turn::call("memory.remember", json!({"content": content, "kind": "preference", "scope": scope}))
}

fn visible() -> Visibility {
    Visibility::tenant(TenantId::local())
}

async fn facts(env: &Env) -> Vec<zorix_core::MemoryItem> {
    env.store
        .list_memory(
            &visible(),
            &MemoryFilter { include_inactive: true, ..Default::default() },
            Timestamp(i64::MAX / 2),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn a_proposal_waits_for_a_person_and_what_is_approved_is_seen_by_the_next_execution() {
    let env = env(vec![
        remember("Always answer in Portuguese.", "global"),
        Turn::Say("Noted.".into()),
        Turn::Say("Olá!".into()),
    ]);
    let waiting = start(&env, new_run("Please remember that I like Portuguese", Some("chat-1"), None)).await;
    assert_eq!(waiting.state, S::WaitingForApproval);
    assert!(facts(&env).await.is_empty(), "nothing is remembered before a person says yes");
    let approval = pending(&env.engine, waiting.id).await;
    assert_eq!(
        (approval.tool.as_str(), approval.arguments["content"].as_str()),
        ("memory.remember", Some("Always answer in Portuguese."))
    );
    let event = env
        .engine
        .events(&TenantId::local(), waiting.id, 0)
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.kind.name() == "execution.waiting_approval")
        .unwrap();
    assert_eq!(event.payload["person_only"], json!(true));

    // An automatic mode cannot approve it.
    let refused = decide(&env.engine, waiting.id, true, true).await;
    assert!(matches!(&refused, Err(Error::InvalidInput(m)) if m.contains("approved by a person")), "{refused:?}");
    assert!(facts(&env).await.is_empty());

    decide(&env.engine, waiting.id, true, false).await.unwrap();
    assert_eq!(settle(&env.engine, waiting.id).await.state, S::Completed);
    let stored = facts(&env).await;
    assert_eq!(stored.len(), 1);
    let MemoryOrigin::Model { execution_id, approved_by, approval_id, .. } = &stored[0].origin else {
        panic!("a model origin")
    };
    assert_eq!(
        (*execution_id, approved_by.as_str(), *approval_id),
        (waiting.id, "luis", Some(approval.id)),
        "who proposed and who approved is on the record"
    );

    // A later execution, in another conversation, is told.
    start(&env, new_run("Say hello", Some("chat-2"), None)).await;
    let later = &env.model.requests()[2];
    let message =
        later.messages.iter().find(|m| m.content.starts_with("Remembered")).expect("the fact is in the context");
    assert!(message.content.contains("Always answer in Portuguese.") && message.content.contains("[m:"));
}

#[tokio::test]
async fn a_refused_proposal_stores_nothing_and_the_model_is_told() {
    let env = env(vec![
        remember("The password is hunter2", "global"),
        Turn::Say("Understood, I will not remember it.".into()),
    ]);
    let waiting = start(&env, new_run("remember this", None, None)).await;
    let approval = pending(&env.engine, waiting.id).await;
    let no = ApprovalDecision {
        approve: false,
        decided_by: "luis".into(),
        note: Some("no secrets".into()),
        call_hash: Some(approval.call_hash),
        automatic: false,
    };
    env.engine.decide(&TenantId::local(), waiting.id, approval.id, no).await.unwrap();
    assert_eq!(settle(&env.engine, waiting.id).await.state, S::Completed);
    assert!(facts(&env).await.is_empty());
    assert!(
        env.model.requests()[1].messages.iter().any(|m| m.content.contains("no secrets")),
        "the note reaches the model"
    );
}

#[tokio::test]
async fn text_that_looks_like_a_credential_is_refused_even_when_approved() {
    let env = env(vec![
        remember("Deploy with key sk-abcdefghijklmnopqrstuvwxyz0123456789", "global"),
        Turn::Say("ok".into()),
    ]);
    let waiting = start(&env, new_run("remember", None, None)).await;
    decide(&env.engine, waiting.id, true, false).await.unwrap();
    settle(&env.engine, waiting.id).await;
    assert!(facts(&env).await.is_empty());
    let detail = env.engine.detail(&TenantId::local(), waiting.id).await.unwrap();
    assert!(
        detail.tool_calls[0].error.as_deref().is_some_and(|e| e.contains("credential")),
        "{:?}",
        detail.tool_calls[0].error
    );
}

#[tokio::test]
async fn scopes_need_what_they_name_and_stay_where_they_were_put() {
    let env = env(vec![
        remember("Only here.", "project"),
        Turn::Say("a".into()),
        remember("Only this chat.", "conversation"),
        Turn::Say("b".into()),
        remember("Project fact.", "project"),
        Turn::Say("c".into()),
        remember("Chat fact.", "conversation"),
        Turn::Say("d".into()),
        Turn::Say("e".into()),
        Turn::Say("f".into()),
    ]);
    // No workspace: a project memory has nowhere to live.
    let no_project = start(&env, new_run("one", Some("chat-a"), None)).await;
    decide(&env.engine, no_project.id, true, false).await.unwrap();
    settle(&env.engine, no_project.id).await;
    // No conversation: same for a conversation memory.
    let no_chat = start(&env, new_run("two", None, Some("/work/one"))).await;
    decide(&env.engine, no_chat.id, true, false).await.unwrap();
    settle(&env.engine, no_chat.id).await;
    assert!(facts(&env).await.is_empty());
    // With both, they are stored and are visible only to their own project and conversation.
    let run = start(&env, new_run("three", Some("chat-a"), Some("/work/one"))).await;
    decide(&env.engine, run.id, true, false).await.unwrap();
    settle(&env.engine, run.id).await;
    let run = start(&env, new_run("four", Some("chat-a"), Some("/work/one"))).await;
    decide(&env.engine, run.id, true, false).await.unwrap();
    settle(&env.engine, run.id).await;
    assert_eq!(facts(&env).await.len(), 0, "global visibility alone sees neither");
    let both = Visibility {
        tenant: TenantId::local(),
        project: Some("/work/one".into()),
        conversation: Some(ConversationId::new("chat-a").unwrap()),
    };
    let seen = env.store.list_memory(&both, &MemoryFilter::default(), Timestamp(i64::MAX / 2)).await.unwrap();
    let mut contents: Vec<&str> = seen.iter().map(|i| i.content.as_str()).collect();
    contents.sort();
    assert_eq!(contents, ["Chat fact.", "Project fact."]);
    let other_project = Visibility { project: Some("/work/two".into()), ..both.clone() };
    assert_eq!(
        env.store.list_memory(&other_project, &MemoryFilter::default(), Timestamp(i64::MAX / 2)).await.unwrap().len(),
        1,
        "only the conversation's"
    );
}

#[tokio::test]
async fn search_finds_what_was_approved_and_needs_no_approval() {
    let env = env(vec![
        remember("The database is Postgres 15.", "global"),
        Turn::Say("saved".into()),
        Turn::call("memory.search", json!({"query": "which database"})),
        Turn::Say("found".into()),
    ]);
    let first = start(&env, new_run("remember the database", None, None)).await;
    decide(&env.engine, first.id, true, false).await.unwrap();
    settle(&env.engine, first.id).await;
    // A read needs no approval, and returns the id and when it was confirmed.
    let search = start(&env, new_run("what database do we use?", None, None)).await;
    assert_eq!(search.state, S::Completed, "no approval for a read");
    let result = env.model.requests()[3]
        .messages
        .iter()
        .find(|m| m.content.starts_with("<tool_result"))
        .expect("the search result")
        .content
        .clone();
    assert!(
        result.contains("Postgres 15") && result.contains("\"id\":\"m:") && result.contains("confirmed"),
        "{result}"
    );
}

/// An environment where a fact already exists, and the script is built from its id.
async fn env_with_fact(content: &str, script: impl FnOnce(MemoryId) -> Vec<Turn>) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zorix.sqlite");
    let repository = Arc::new(SqliteRepository::open(&path).unwrap());
    let store = Arc::new(SqliteMemoryStore::open(&path).unwrap());
    let old = store
        .remember(
            &TenantId::local(),
            zorix_core::NewMemory {
                scope: zorix_core::Scope::Global,
                kind: zorix_core::MemoryKind::Fact,
                subject: None,
                content: content.into(),
                valid_until: None,
                origin: MemoryOrigin::User { name: "t".into() },
                replaces: vec![],
            },
            Timestamp(1),
        )
        .await
        .unwrap()
        .item()
        .id;
    let stack = MemoryStack::new(store.clone(), repository.clone(), None);
    let registry = Arc::new(InMemoryToolRegistry::new());
    stack.register_tools(&*registry).unwrap();
    let model = Arc::new(ScriptedModel::new(script(old)));
    let engine = stack
        .install(Engine::builder(repository, model.clone()).tools(registry))
        .config(EngineConfig::default())
        .build();
    Env { _dir: dir, store, engine, model }
}

#[tokio::test]
async fn a_replacement_by_id_marks_the_old_one_as_replaced_and_keeps_it_for_the_record() {
    let env = env_with_fact("The database is Postgres 15.", |old| {
        vec![Turn::call("memory.remember", json!({"content": "The database is Postgres 16.", "kind": "fact", "scope": "global", "replaces": [format!("m:{old}")]})), Turn::Say("done".into())]
    })
    .await;
    let run = start(&env, new_run("update the database fact", None, None)).await;
    decide(&env.engine, run.id, true, false).await.unwrap();
    settle(&env.engine, run.id).await;
    let after = facts(&env).await;
    assert_eq!(after.len(), 2, "both are kept");
    let old = after.iter().find(|i| i.content.contains("15")).unwrap();
    let new = after.iter().find(|i| i.content.contains("16")).unwrap();
    assert_eq!((old.status, old.superseded_by), (MemoryStatus::Superseded, Some(new.id)));
    assert_eq!(new.status, MemoryStatus::Active);
}

#[tokio::test]
async fn replacing_something_that_is_not_visible_to_the_run_is_refused() {
    let env = env(vec![
        Turn::call(
            "memory.remember",
            json!({"content": "A fix.", "kind": "fact", "scope": "global", "replaces": [format!("m:{}", MemoryId::new())]}),
        ),
        Turn::Say("ok".into()),
    ]);
    let run = start(&env, new_run("fix it", None, None)).await;
    decide(&env.engine, run.id, true, false).await.unwrap();
    settle(&env.engine, run.id).await;
    assert!(facts(&env).await.is_empty());
    let detail = env.engine.detail(&TenantId::local(), run.id).await.unwrap();
    assert!(
        detail.tool_calls[0].error.as_deref().is_some_and(|e| e.contains("not a memory this run can replace")),
        "{:?}",
        detail.tool_calls[0].error
    );
}

#[tokio::test]
async fn a_past_or_invalid_expiry_is_refused_and_a_future_one_is_kept() {
    let env = env(vec![
        Turn::call(
            "memory.remember",
            json!({"content": "Freeze ends soon.", "kind": "fact", "scope": "global", "valid_until": "2001-01-01"}),
        ),
        Turn::Say("a".into()),
        Turn::call(
            "memory.remember",
            json!({"content": "Freeze ends soon.", "kind": "fact", "scope": "global", "valid_until": "2027-02-31"}),
        ),
        Turn::Say("b".into()),
        Turn::call(
            "memory.remember",
            json!({"content": "Freeze ends later.", "kind": "fact", "scope": "global", "valid_until": "2099-12-31"}),
        ),
        Turn::Say("c".into()),
    ]);
    for prompt in ["one", "two", "three"] {
        let run = start(&env, new_run(prompt, None, None)).await;
        decide(&env.engine, run.id, true, false).await.unwrap();
        settle(&env.engine, run.id).await;
    }
    let all = facts(&env).await;
    assert_eq!(all.len(), 1);
    assert_eq!((all[0].content.as_str(), all[0].valid_until.is_some()), ("Freeze ends later.", true));
}

#[tokio::test]
async fn an_execution_cannot_propose_more_than_ten_memories() {
    let mut turns: Vec<Turn> = (0..11)
        .map(|i| {
            Turn::call(
                "memory.remember",
                json!({"content": format!("Preference number {i}."), "kind": "preference", "scope": "global"}),
            )
        })
        .collect();
    turns.push(Turn::Say("done".into()));
    let env = env(turns);
    let mut new = new_run("remember a lot", None, None);
    new.limits.max_iterations = 40;
    new.limits.max_tool_calls = 40;
    let mut run = start(&env, new).await;
    for _ in 0..11 {
        assert_eq!(run.state, S::WaitingForApproval);
        decide(&env.engine, run.id, true, false).await.unwrap();
        run = settle(&env.engine, run.id).await;
    }
    assert_eq!(run.state, S::Completed);
    assert_eq!(facts(&env).await.len(), 10);
    let detail = env.engine.detail(&TenantId::local(), run.id).await.unwrap();
    assert!(
        detail.tool_calls.last().unwrap().error.as_deref().is_some_and(|e| e.contains("ten")),
        "{:?}",
        detail.tool_calls.last().unwrap().error
    );
}

#[tokio::test]
async fn memory_tools_are_unavailable_to_a_run_that_was_not_granted_them() {
    let env = env(vec![Turn::call("memory.search", json!({})), Turn::Say("nothing granted".into())]);
    let mut new = new_run("search", None, None);
    new.grants = Grants::none();
    let run = start(&env, new).await;
    assert_eq!(run.state, S::Completed);
    let detail = env.engine.detail(&TenantId::local(), run.id).await.unwrap();
    assert!(detail.tool_calls.iter().all(|c| c.status != zorix_core::ToolCallStatus::Succeeded), "denied by default");
}

#[tokio::test]
async fn a_search_sees_only_this_tenants_memory() {
    let env = env(vec![Turn::call("memory.search", json!({"query": "plan"})), Turn::Say("looked".into())]);
    let other = TenantId::new("other").unwrap();
    env.store
        .remember(
            &other,
            zorix_core::NewMemory {
                scope: zorix_core::Scope::Global,
                kind: zorix_core::MemoryKind::Fact,
                subject: None,
                content: "The other tenant's secret plan.".into(),
                valid_until: None,
                origin: MemoryOrigin::User { name: "t".into() },
                replaces: vec![],
            },
            Timestamp(1),
        )
        .await
        .unwrap();
    let run = start(&env, new_run("search the plan", None, None)).await;
    assert_eq!(run.state, S::Completed);
    let result = env.model.requests()[1]
        .messages
        .iter()
        .find(|m| m.content.starts_with("<tool_result"))
        .unwrap()
        .content
        .clone();
    assert!(result.contains("\"count\":0") && !result.contains("secret plan"), "{result}");
}
