//! Creation, the lifecycle, cancellation, pause, events, tenants and idempotency.

mod common;

use common::*;
use futures_util::StreamExt;
use zorix_core::{Error, ExecutionEngine, ExecutionState as S, ListFilter, TenantId};
use zorix_sim::{ScriptedModel, Turn};

#[tokio::test]
async fn creating_an_execution_stores_it_without_running_it() {
    let h = harness(vec![]);
    let created = h.engine.create(request("hi", &[])).await.unwrap();
    assert_eq!(created.state, S::Created);
    assert_eq!(created.version, 1);
    assert_eq!(kinds(&h.engine, created.id).await, vec!["execution.created"]);
    assert_eq!(h.engine.get(&local(), created.id).await.unwrap().id, created.id);
    assert_eq!(h.model.requests().len(), 0, "nothing ran");
}

#[tokio::test]
async fn bad_requests_are_refused_before_anything_is_stored() {
    let h = harness(vec![]);
    let error = h.engine.create(request("   ", &[])).await.unwrap_err();
    assert!(matches!(error, Error::InvalidInput(_)), "{error}");
    assert!(h.engine.list(&local(), &ListFilter::default()).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_run_without_tools_follows_the_documented_lifecycle() {
    let h = harness(vec![Turn::Say("Hello!".into())]);
    let done = run(&h.engine, request("Say hello", &[])).await;
    assert_eq!(done.state, S::Completed);
    assert_eq!(done.result.as_ref().unwrap().text, "Hello!");
    assert!(done.started_at.is_some() && done.finished_at.is_some());
    assert_eq!(
        kinds(&h.engine, done.id).await,
        vec![
            "execution.created",
            "execution.queued",
            "execution.started",
            "execution.model.requested",
            "execution.model.responded",
            "execution.completed"
        ]
    );
    assert!(done.usage.total_tokens() > 0 && done.usage.model_calls == 1 && done.usage.cost_micros > 0);
}

#[tokio::test]
async fn steps_the_lifecycle_does_not_allow_are_refused() {
    let h = harness(vec![Turn::Say("done".into())]);
    let created = h.engine.create(request("x", &[])).await.unwrap();
    for result in [h.engine.resume(&local(), created.id).await, h.engine.pause(&local(), created.id).await] {
        assert!(matches!(result, Err(Error::InvalidTransition(_))), "{result:?}");
    }
    let done = run(&h.engine, request("y", &[])).await;
    assert_eq!(done.state, S::Completed);
    assert!(matches!(h.engine.cancel(&local(), done.id, None).await, Err(Error::InvalidTransition(_))));
    assert!(matches!(h.engine.start(&local(), done.id).await, Err(Error::InvalidTransition(_))));
    assert!(matches!(h.engine.pause(&local(), done.id).await, Err(Error::InvalidTransition(_))));
    assert!(matches!(h.engine.resume(&local(), done.id).await, Err(Error::InvalidTransition(_))));
}

#[tokio::test]
async fn starting_twice_does_not_run_twice() {
    let h = harness(vec![Turn::Say("once".into())]);
    let created = h.engine.create(request("x", &[])).await.unwrap();
    h.engine.start(&local(), created.id).await.unwrap();
    h.engine.start(&local(), created.id).await.unwrap();
    let done = settled(&h.engine, created.id).await;
    assert_eq!(done.state, S::Completed);
    assert_eq!(h.model.requests().len(), 1);
}

#[tokio::test]
async fn the_same_idempotency_key_returns_the_same_execution_and_runs_once() {
    let h = harness(vec![Turn::Say("once".into())]);
    let mut first = request("pay the invoice", &[]);
    first.idempotency_key = Some("invoice-42".into());
    let a = h.engine.submit(first.clone()).await.unwrap();
    let b = h.engine.submit(first).await.unwrap();
    assert_eq!(a.id, b.id);
    settled(&h.engine, a.id).await;
    assert_eq!(h.model.requests().len(), 1);
    assert_eq!(h.engine.list(&local(), &ListFilter::default()).await.unwrap().len(), 1);
}

#[tokio::test]
async fn an_execution_is_not_a_conversation_so_one_conversation_can_have_many() {
    let h = harness(vec![Turn::Say("a".into()), Turn::Say("b".into())]);
    let conversation = zorix_core::ConversationId::new("chat-1").unwrap();
    let mut one = request("first", &[]);
    one.conversation = Some(conversation.clone());
    let mut two = request("second", &[]);
    two.conversation = Some(conversation.clone());
    let a = run(&h.engine, one).await;
    let b = run(&h.engine, two).await;
    assert_ne!(a.id, b.id);
    let listed = h
        .engine
        .list(&local(), &ListFilter { conversation: Some(conversation), ..ListFilter::default() })
        .await
        .unwrap();
    assert_eq!(listed.len(), 2);
}

#[tokio::test]
async fn cancelling_stops_a_model_call_in_flight() {
    let h = harness(vec![Turn::Hang]);
    let started = h.engine.submit(request("wait forever", &[])).await.unwrap();
    wait_for_state(&h.engine, started.id, S::WaitingForModel).await;
    let cancelled = h.engine.cancel(&local(), started.id, Some("changed my mind".into())).await.unwrap();
    assert_eq!(cancelled.state, S::Cancelled);
    assert_eq!(cancelled.cancel_reason.as_deref(), Some("changed my mind"));
    let all = kinds(&h.engine, started.id).await;
    assert_eq!(all.last(), Some(&"execution.cancelled"));
    assert!(!all.contains(&"execution.completed") && !all.contains(&"execution.failed"));
}

#[tokio::test]
async fn cancelling_stops_a_tool_that_is_running() {
    let h = harness(vec![Turn::call("slow", serde_json::json!({}))]);
    let started = h.engine.submit(request("do the slow thing", &["slow"])).await.unwrap();
    wait_for_state(&h.engine, started.id, S::WaitingForTool).await;
    assert_eq!(h.engine.cancel(&local(), started.id, None).await.unwrap().state, S::Cancelled);
    assert_eq!(h.slow.count(), 1);
}

#[tokio::test]
async fn cancelling_before_the_start_works_and_a_second_cancel_is_refused() {
    let h = harness(vec![]);
    let created = h.engine.create(request("x", &[])).await.unwrap();
    assert_eq!(h.engine.cancel(&local(), created.id, None).await.unwrap().state, S::Cancelled);
    assert!(matches!(h.engine.cancel(&local(), created.id, None).await, Err(Error::InvalidTransition(_))));
    assert!(matches!(h.engine.start(&local(), created.id).await, Err(Error::InvalidTransition(_))));
}

#[tokio::test]
async fn pausing_stops_at_the_next_step_and_resuming_continues_without_repeating_work() {
    let h = harness(vec![
        Turn::Delay { ms: 150, then: Box::new(Turn::call("echo", serde_json::json!({"text": "once"}))) },
        Turn::Say("finished".into()),
    ]);
    let started = h.engine.submit(request("go", &["echo"])).await.unwrap();
    wait_for_state(&h.engine, started.id, S::WaitingForModel).await;
    h.engine.pause(&local(), started.id).await.unwrap();
    let paused = settled(&h.engine, started.id).await;
    assert_eq!(paused.state, S::Paused);
    assert_eq!(h.echo.count(), 0, "the step boundary came before the tool");

    h.engine.resume(&local(), started.id).await.unwrap();
    let done = settled(&h.engine, started.id).await;
    assert_eq!(done.state, S::Completed);
    assert_eq!(h.echo.count(), 1);
    assert_eq!(h.model.requests().len(), 2);
    let all = kinds(&h.engine, started.id).await;
    assert!(in_order(&all, &["execution.paused", "execution.resumed", "execution.completed"]), "{all:?}");
}

#[tokio::test]
async fn events_are_numbered_in_order_and_a_subscriber_sees_the_same_ones_live() {
    let h = harness(vec![Turn::call("echo", serde_json::json!({"text": "x"})), Turn::Say("ok".into())]);
    let created = h.engine.create(request("go", &["echo"])).await.unwrap();
    let stream = h.engine.subscribe(&local(), created.id, 0).await.unwrap();
    let collector = tokio::spawn(stream.collect::<Vec<_>>());
    h.engine.start(&local(), created.id).await.unwrap();
    let live = tokio::time::timeout(std::time::Duration::from_secs(10), collector).await.unwrap().unwrap();
    let stored = h.engine.events(&local(), created.id, 0).await.unwrap();
    assert_eq!(live, stored, "live and stored agree, with no gaps or repeats");
    assert!(stored.windows(2).all(|w| w[0].seq < w[1].seq));
    let all: Vec<&str> = stored.iter().map(|e| e.kind.name()).collect();
    assert!(
        in_order(
            &all,
            &[
                "execution.created",
                "execution.queued",
                "execution.started",
                "execution.model.requested",
                "execution.model.responded",
                "execution.tool.requested",
                "execution.tool.started",
                "execution.tool.completed",
                "execution.model.requested",
                "execution.model.responded",
                "execution.completed"
            ]
        ),
        "{all:?}"
    );
}

#[tokio::test]
async fn a_late_subscriber_resumes_after_the_last_event_it_saw() {
    let h = harness(vec![Turn::Say("ok".into())]);
    let done = run(&h.engine, request("go", &[])).await;
    let stored = h.engine.events(&local(), done.id, 0).await.unwrap();
    let tail: Vec<_> = h.engine.subscribe(&local(), done.id, stored[2].seq).await.unwrap().collect().await;
    assert_eq!(tail, stored[3..].to_vec());
}

#[tokio::test]
async fn another_tenant_cannot_see_or_touch_an_execution() {
    let h = harness(vec![Turn::Say("ok".into())]);
    let mut mine = request("secret", &[]);
    mine.tenant = Some(TenantId::new("acme").unwrap());
    let created = h.engine.create(mine).await.unwrap();
    let other = TenantId::new("globex").unwrap();
    let id = created.id;
    assert!(matches!(h.engine.get(&other, id).await, Err(Error::NotFound(_))));
    assert!(matches!(h.engine.start(&other, id).await, Err(Error::NotFound(_))));
    assert!(matches!(h.engine.cancel(&other, id, None).await, Err(Error::NotFound(_))));
    assert!(matches!(h.engine.events(&other, id, 0).await, Err(Error::NotFound(_))));
    assert!(matches!(h.engine.detail(&other, id).await, Err(Error::NotFound(_))));
    assert!(h.engine.list(&other, &ListFilter::default()).await.unwrap().is_empty());
    assert_eq!(h.engine.get(&TenantId::new("acme").unwrap(), id).await.unwrap().id, id);
}

#[tokio::test]
async fn a_second_engine_on_the_same_store_does_not_run_an_execution_twice() {
    use std::sync::Arc;
    let repo = Arc::new(zorix_runtime::MemoryRepository::new());
    let first_model = Arc::new(ScriptedModel::new([Turn::Delay { ms: 200, then: Box::new(Turn::Say("mine".into())) }]));
    let second_model = Arc::new(ScriptedModel::new([Turn::Say("theirs".into())]));
    let first = zorix_runtime::Engine::builder(repo.clone(), first_model.clone()).config(fast_config()).build();
    let second = zorix_runtime::Engine::builder(repo.clone(), second_model.clone()).config(fast_config()).build();

    let started = first.submit(request("x", &[])).await.unwrap();
    wait_for_state(&first, started.id, S::WaitingForModel).await;
    // The second engine finds work that looks interrupted and offers it to its own workers.
    let report = second.recover().await.unwrap();
    assert_eq!((report.in_use, report.requeued.len()), (1, 0), "it is held by a live engine");
    let done = settled(&first, started.id).await;
    assert_eq!(done.result.unwrap().text, "mine");
    assert_eq!(second_model.requests().len(), 0, "the other engine never ran it");
    assert_eq!(first_model.requests().len(), 1);
}
