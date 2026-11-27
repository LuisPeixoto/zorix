//! Retention on a real database, with runs made by a real engine: a compacted run keeps its request, answer, usage and times and loses its
//! events, steps, approvals and recall; a deleted run is gone unless something refers to it; a tree is never cut in half; nothing that is still
//! moving is touched; and a preview counts exactly what a pass then does.

use std::sync::Arc;
use std::time::Duration;

use rusqlite::Connection;
use zorix_core::{
    Clock, ExecutionEngine, ExecutionId, ExecutionRepository, ExecutionState as S, Grants, ListFilter, NewExecution,
    Permission, RetentionMode, TaskRequest, TenantId, Timestamp,
};
use zorix_runtime::{Engine, EngineConfig, InMemoryToolRegistry};
use zorix_sim::DemoModel;
use zorix_store::{SqliteRepository, SqliteRetention};
use zorix_tools::{BuiltinConfig, register_builtin};

struct World {
    dir: tempfile::TempDir,
    engine: Engine,
    repository: Arc<SqliteRepository>,
    retention: SqliteRetention,
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zorix.sqlite");
    let registry = Arc::new(InMemoryToolRegistry::new());
    register_builtin(&*registry, &BuiltinConfig { notes_file: dir.path().join("notes.json") }).unwrap();
    let repository = Arc::new(SqliteRepository::open(&path).unwrap());
    let config = EngineConfig { sole_owner: true, lease_ttl_ms: 1_000, ..EngineConfig::default() };
    let engine = Engine::builder(repository.clone(), Arc::new(DemoModel::new())).tools(registry).config(config).build();
    let retention = SqliteRetention::open(&path).unwrap();
    World { dir, engine, repository, retention }
}

fn tenant() -> TenantId {
    TenantId::local()
}

async fn run(world: &World, prompt: &str, approve_notes: bool) -> zorix_core::Execution {
    let grants = Grants::none().with(Permission::use_tool("math.evaluate")).with(Permission::use_tool("notes.put"));
    let new = NewExecution {
        request: TaskRequest { prompt: prompt.into(), model: "demo".into(), ..TaskRequest::default() },
        grants,
        ..NewExecution::default()
    };
    let started = world.engine.submit(new).await.unwrap();
    let waiting = tokio::time::timeout(Duration::from_secs(10), world.engine.settle(&tenant(), started.id))
        .await
        .unwrap()
        .unwrap();
    if waiting.state == S::WaitingForApproval && approve_notes {
        let detail = world.engine.detail(&tenant(), started.id).await.unwrap();
        let approval =
            detail.approvals.iter().find(|a| a.status == zorix_core::ApprovalStatus::Pending).unwrap().clone();
        let decision = zorix_core::ApprovalDecision {
            approve: true,
            decided_by: "luis".into(),
            note: None,
            call_hash: Some(approval.call_hash),
            automatic: false,
        };
        world.engine.decide(&tenant(), started.id, approval.id, decision).await.unwrap();
        return tokio::time::timeout(Duration::from_secs(10), world.engine.settle(&tenant(), started.id))
            .await
            .unwrap()
            .unwrap();
    }
    waiting
}

fn count(world: &World, sql: &str) -> i64 {
    Connection::open(world.dir.path().join("zorix.sqlite")).unwrap().query_row(sql, [], |r| r.get(0)).unwrap()
}

fn later() -> Timestamp {
    Timestamp(zorix_core::SystemClock.now().millis() + 86_400_000)
}

#[tokio::test]
async fn compacting_keeps_the_run_and_its_numbers_and_drops_what_is_bulky() {
    let w = world();
    let done = run(&w, "What is 17*23 and save it as result", true).await;
    assert_eq!(done.state, S::Completed);
    let before = w.repository.get(&tenant(), done.id).await.unwrap().unwrap();
    assert!(count(&w, "SELECT COUNT(*) FROM events") > 5 && count(&w, "SELECT COUNT(*) FROM tool_calls") == 2);

    let report = w.retention.apply(&tenant(), later(), RetentionMode::Compact).await.unwrap();
    assert_eq!((report.runs, report.compacted, report.deleted), (1, 1, 0));
    assert!(report.events > 5 && report.tool_calls == 2 && report.approvals == 1 && report.bytes > 0, "{report:?}");

    for table in ["events", "checkpoints", "tool_calls", "approvals", "context_snapshots", "snapshot_items", "leases"] {
        assert_eq!(count(&w, &format!("SELECT COUNT(*) FROM {table}")), 0, "{table} should be empty");
    }
    // The run is still there, still says what it asked and answered, and its numbers are exactly what they were.
    let after = w.repository.get(&tenant(), done.id).await.unwrap().expect("the run stays");
    assert!(after.compacted && !before.compacted);
    assert_eq!(after.request, before.request);
    assert_eq!(after.result, before.result);
    assert_eq!(
        (after.usage, after.created_at, after.finished_at),
        (before.usage, before.created_at, before.finished_at)
    );
    assert_eq!(w.repository.list(&tenant(), &ListFilter::default()).await.unwrap().len(), 1, "it is still listed");

    // A second pass has nothing to do.
    assert_eq!(w.retention.apply(&tenant(), later(), RetentionMode::Compact).await.unwrap(), Default::default());
}

#[tokio::test]
async fn a_preview_counts_what_a_pass_then_does_and_changes_nothing() {
    let w = world();
    run(&w, "What is 17*23 and save it as result", true).await;
    run(&w, "What is 17*23 and save it as result", true).await;
    let events = count(&w, "SELECT COUNT(*) FROM events");

    let preview = w.retention.preview(&tenant(), later(), RetentionMode::Compact).await.unwrap();
    assert_eq!(count(&w, "SELECT COUNT(*) FROM events"), events, "a preview changed something");
    assert_eq!(count(&w, "SELECT COUNT(*) FROM executions WHERE compacted = 1"), 0);
    let applied = w.retention.apply(&tenant(), later(), RetentionMode::Compact).await.unwrap();
    assert_eq!(applied, preview);
    assert_eq!(applied.runs, 2);
}

#[tokio::test]
async fn nothing_that_is_waiting_or_not_old_enough_is_touched() {
    let w = world();
    // One that finished, and one that is waiting for a person.
    run(&w, "What is 17*23 and save it as result", true).await;
    let waiting = run(&w, "What is 17*23 and save it as result", false).await;
    assert_eq!(waiting.state, S::WaitingForApproval);
    let report = w.retention.apply(&tenant(), later(), RetentionMode::Delete).await.unwrap();
    assert_eq!((report.runs, report.deleted), (1, 1), "only the finished one: {report:?}");
    assert_eq!(count(&w, "SELECT COUNT(*) FROM executions"), 1);
    assert!(
        count(&w, "SELECT COUNT(*) FROM events WHERE execution_id = (SELECT id FROM executions)") > 0,
        "the waiting run keeps its events"
    );

    // Nothing is old enough for a cutoff in the past.
    let w = world();
    run(&w, "What is 17*23 and save it as result", true).await;
    assert_eq!(w.retention.apply(&tenant(), Timestamp(0), RetentionMode::Delete).await.unwrap(), Default::default());
    assert_eq!(count(&w, "SELECT COUNT(*) FROM executions"), 1);
}

#[tokio::test]
async fn delete_removes_the_run_and_its_past_work_unless_something_refers_to_it() {
    let w = world();
    let mine = run(&w, "What is 17*23 and save it as result", true).await;
    let kept = run(&w, "What is 17*23 and save it as result", true).await;
    // The past work of both runs is recorded in the background.
    for _ in 0..200 {
        if count(&w, "SELECT COUNT(*) FROM operations") == 0 {
            break; // this world has no recorder (no memory stack): nothing to wait for
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // A task of a project says it ran `kept`.
    let conn = Connection::open(w.dir.path().join("zorix.sqlite")).unwrap();
    conn.execute(
        "INSERT INTO tasks (id, tenant, project_id, key, state, rank, revision, updated_at, doc) VALUES ('t1','local','p1','OPS-1','done',1,1,1,?1)",
        [format!(r#"{{"runs":[{{"execution":"{}"}}]}}"#, kept.id)],
    )
    .unwrap();
    for id in [mine.id, kept.id] {
        conn.execute(
            "INSERT OR REPLACE INTO operations (execution_id, tenant, project, conversation, state, finished_at, text, text_hash, doc) VALUES (?1,'local',NULL,NULL,'completed',1,'what was done','h','{}')",
            [id.to_string()],
        )
        .unwrap();
    }
    drop(conn);

    let report = w.retention.apply(&tenant(), later(), RetentionMode::Delete).await.unwrap();
    assert_eq!(
        (report.runs, report.deleted, report.compacted, report.kept_because_referenced),
        (2, 1, 1, 1),
        "{report:?}"
    );
    assert_eq!(
        count(&w, "SELECT COUNT(*) FROM operations"),
        1,
        "the deleted run's past work went with it, the other's stays"
    );
    assert!(w.repository.get(&tenant(), mine.id).await.unwrap().is_none(), "the unreferenced run is gone");
    let still = w.repository.get(&tenant(), kept.id).await.unwrap().expect("the referenced run stays");
    assert!(still.compacted, "and is compacted instead");
}

#[tokio::test]
async fn a_tree_is_never_cut_in_half() {
    let w = world();
    let parent = run(&w, "What is 17*23 and save it as result", true).await;
    let child = run(&w, "What is 17*23 and save it as result", true).await;
    // Make them a tree: the child's root is the parent, and the child finished a day later than the parent.
    let conn = Connection::open(w.dir.path().join("zorix.sqlite")).unwrap();
    conn.execute(
        "UPDATE executions SET root = ?1, parent = ?1 WHERE id = ?2",
        [parent.id.to_string(), child.id.to_string()],
    )
    .unwrap();
    let now = zorix_core::SystemClock.now().millis();
    conn.execute(
        "UPDATE executions SET doc = json_set(doc, '$.finished_at', ?1) WHERE id = ?2",
        rusqlite::params![now + 86_400_000, child.id.to_string()],
    )
    .unwrap();
    drop(conn);
    // A cutoff between the two: the parent alone would qualify, but its child is newer, so neither does.
    let between = Timestamp(now + 3_600_000);
    assert_eq!(w.retention.apply(&tenant(), between, RetentionMode::Compact).await.unwrap(), Default::default());
    assert_eq!(count(&w, "SELECT COUNT(*) FROM executions WHERE compacted = 1"), 0);
    // Later, both go together.
    let report = w.retention.apply(&tenant(), Timestamp(now + 2 * 86_400_000), RetentionMode::Compact).await.unwrap();
    assert_eq!(report.runs, 2);
    let _: ExecutionId = parent.id;
}

#[tokio::test]
async fn a_pass_follows_the_settings_and_is_off_without_a_number_of_days() {
    use zorix_core::{AppSettings, RetentionSettings};

    let w = world();
    run(&w, "What is 17*23 and save it as result", true).await;
    let now = Timestamp(zorix_core::SystemClock.now().millis());
    let off = AppSettings::default();
    assert_eq!(w.retention.pass(&tenant(), &off, now, false).await.unwrap(), None, "off keeps everything for ever");
    let on = AppSettings {
        retention: RetentionSettings { days: Some(30), mode: RetentionMode::Compact },
        ..AppSettings::default()
    };
    // Today the run is not 30 days old; in 31 days it is.
    assert_eq!(w.retention.pass(&tenant(), &on, now, false).await.unwrap().unwrap().runs, 0);
    let in_a_month = Timestamp(now.millis() + 31 * 86_400_000);
    let preview = w.retention.pass(&tenant(), &on, in_a_month, true).await.unwrap().unwrap();
    assert_eq!(count(&w, "SELECT COUNT(*) FROM executions WHERE compacted = 1"), 0, "a dry pass changed something");
    let done = w.retention.pass(&tenant(), &on, in_a_month, false).await.unwrap().unwrap();
    assert_eq!((done.runs, done.compacted), (1, 1));
    assert_eq!(done, preview);
}
