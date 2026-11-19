//! Live text reaches whoever is watching one execution, never another, and is not stored.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use futures_util::StreamExt;
use zorix_core::{ExecutionEngine, ExecutionId, LiveEvent, TenantId};
use zorix_runtime::{Engine, InMemoryToolRegistry, MemoryRepository};
use zorix_sim::{ScriptedModel, Turn};

fn engine(turns: Vec<Turn>) -> Engine {
    Engine::builder(Arc::new(MemoryRepository::new()), Arc::new(ScriptedModel::new(turns)))
        .tools(Arc::new(InMemoryToolRegistry::new()))
        .config(fast_config())
        .build()
}

async fn drain(stream: &mut futures_util::stream::BoxStream<'static, LiveEvent>) -> Vec<LiveEvent> {
    let mut seen = Vec::new();
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_millis(150), stream.next()).await {
        seen.push(event);
    }
    seen
}

#[tokio::test]
async fn a_watcher_sees_the_text_of_its_execution_and_the_record_is_unchanged() {
    let engine = engine(vec![Turn::Say("Hello, live.".into()), Turn::Say("Other run.".into())]);
    let mine = engine.create(request("hi", &[])).await.unwrap();
    let theirs = engine.create(request("hi", &[])).await.unwrap();
    let mut watching = engine.live(&local(), mine.id).await.unwrap();

    engine.start(&local(), mine.id).await.unwrap();
    let done = settled(&engine, mine.id).await;
    let seen = drain(&mut watching).await;
    assert_eq!(seen, vec![LiveEvent::Text { execution: mine.id, delta: "Hello, live.".into() }]);
    assert_eq!(done.result.as_ref().map(|o| o.text.as_str()), Some("Hello, live."));

    // Another execution's text never shows up on this stream.
    engine.start(&local(), theirs.id).await.unwrap();
    settled(&engine, theirs.id).await;
    assert!(drain(&mut watching).await.is_empty());

    // Nothing live was stored.
    let kinds = kinds(&engine, mine.id).await;
    assert!(kinds.iter().all(|k| !k.contains("live")), "{kinds:?}");
}

#[tokio::test]
async fn watching_needs_an_execution_of_the_same_tenant() {
    let engine = engine(vec![]);
    let made = engine.create(request("hi", &[])).await.unwrap();
    assert!(engine.live(&TenantId::new("someone-else").unwrap(), made.id).await.is_err());
    assert!(engine.live(&local(), ExecutionId::new()).await.is_err());
}
