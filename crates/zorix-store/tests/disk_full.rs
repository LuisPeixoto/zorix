//! What a full disk does: the failure is told in words, nothing that was saved is lost or damaged, and once there is room the work
//! that was interrupted carries on.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use zorix_core::{
    Error, ExecutionEngine, ExecutionRepository, ExecutionState as S, FinishReason, Grants, ModelError, ModelGateway,
    ModelRequest, ModelResponse, NewExecution, TaskRequest, TenantId, Usage,
};
use zorix_runtime::{Engine, EngineConfig, InMemoryToolRegistry, RetryPolicy};
use zorix_store::SqliteRepository;

fn local() -> TenantId {
    TenantId::local()
}

struct Echo;

#[async_trait]
impl ModelGateway for Echo {
    async fn generate(&self, request: ModelRequest, _cancel: CancellationToken) -> Result<ModelResponse, ModelError> {
        Ok(ModelResponse::new(request.model, "Done. ".repeat(40), Vec::new(), Usage::default(), FinishReason::Stop))
    }
}

fn request(n: usize) -> NewExecution {
    NewExecution {
        request: TaskRequest {
            prompt: format!("task number {n} {}", "x".repeat(2_000)),
            model: "sim".into(),
            ..TaskRequest::default()
        },
        grants: Grants::none(),
        ..NewExecution::default()
    }
}

fn engine(repo: SqliteRepository) -> Engine {
    let config = EngineConfig {
        workers: 2,
        lease_ttl_ms: 1_000,
        retry: RetryPolicy { base_delay_ms: 1, max_delay_ms: 5 },
        sole_owner: true,
        ..EngineConfig::default()
    };
    Engine::builder(Arc::new(repo), Arc::new(Echo)).tools(Arc::new(InMemoryToolRegistry::new())).config(config).build()
}

#[tokio::test]
async fn a_full_disk_is_reported_in_words_loses_nothing_and_work_continues_when_there_is_room() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zorix.sqlite");
    let repo = SqliteRepository::open(&path).unwrap();
    let engine = engine(repo.clone());

    // A few runs finish while there is room.
    let mut finished = Vec::new();
    for n in 0..3 {
        let started = engine.submit(request(n)).await.unwrap();
        let done =
            tokio::time::timeout(Duration::from_secs(10), engine.settle(&local(), started.id)).await.unwrap().unwrap();
        assert_eq!(done.state, S::Completed);
        finished.push(done.id);
    }

    // Then the file may not grow. Runs keep coming until the storage says no.
    repo.limit_size(Some(repo.size().unwrap())).unwrap();
    let mut refused = None;
    let mut accepted = Vec::new();
    for n in 3..200 {
        match engine.submit(request(n)).await {
            Ok(started) => accepted.push(started.id),
            Err(error) => {
                refused = Some(error);
                break;
            }
        }
    }
    let error = refused.expect("a file that cannot grow must run out of room");
    assert!(
        matches!(&error, Error::Storage(text) if text.starts_with("The disk is full") && text.contains("Free some space")),
        "{error}"
    );
    // Give the runs that were in flight a moment to hit the wall too, then stop this program as a crash would.
    tokio::time::sleep(Duration::from_millis(300)).await;
    engine.abort();
    drop(engine);

    // Nothing that was saved is lost or damaged.
    let conn = rusqlite::Connection::open(&path).unwrap();
    let check: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap();
    assert_eq!(check, "ok");
    for id in &finished {
        let kept = repo.get(&local(), *id).await.unwrap().expect("a finished run is still there");
        assert_eq!(kept.state, S::Completed);
        assert!(repo.events(*id, 0, 100).await.unwrap().iter().any(|e| e.kind.name() == "execution.completed"));
    }

    // There is room again: a new program recovers, and every run that was accepted reaches the end.
    repo.limit_size(None).unwrap();
    let second = engine_over(path.as_path());
    second.recover().await.unwrap();
    for id in accepted {
        let state = tokio::time::timeout(Duration::from_secs(20), second.settle(&local(), id)).await.unwrap().unwrap();
        assert!(state.state.is_terminal() || state.state == S::Completed, "{:?}", state.state);
        assert_eq!(state.state, S::Completed, "{:?}", state.failure);
    }
    second.abort();
}

fn engine_over(path: &std::path::Path) -> Engine {
    engine(SqliteRepository::open(path).unwrap())
}
