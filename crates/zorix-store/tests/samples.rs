//! The numbers an estimate is made from: the newest finished top-level runs of one agent on one model, compacted ones included.

use rusqlite::Connection;
use zorix_core::{AgentId, TenantId};
use zorix_store::SqliteRepository;

#[allow(clippy::too_many_arguments)]
fn insert(
    conn: &Connection,
    id: &str,
    tenant: &str,
    state: &str,
    agent: Option<&str>,
    parent: Option<&str>,
    model: &str,
    at: i64,
    tokens: (u64, u64, u64),
    calls: u64,
    ms: u64,
) {
    let doc = format!(
        r#"{{"request":{{"model":"{model}"}},"usage":{{"input_tokens":{},"output_tokens":{},"cache_read_tokens":{},"model_calls":{calls},"cost_micros":{}}},"active_ms":{ms}}}"#,
        tokens.0,
        tokens.1,
        tokens.2,
        tokens.0 * 2
    );
    conn.execute(
        "INSERT INTO executions (id, tenant, conversation, state, version, created_at, doc, agent, parent) VALUES (?1,?2,NULL,?3,1,?4,?5,?6,?7)",
        rusqlite::params![id, tenant, state, at, doc, agent, parent],
    )
    .unwrap();
}

#[tokio::test]
async fn the_newest_finished_runs_of_the_same_agent_and_model_are_the_samples() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zorix.sqlite");
    let repo = SqliteRepository::open(&path).unwrap();
    let conn = Connection::open(&path).unwrap();
    let agent = AgentId::new();
    let a = agent.to_string();
    let me = TenantId::local();

    insert(&conn, "r1", "local", "completed", Some(&a), None, "m", 1, (100, 10, 0), 2, 1_000);
    insert(&conn, "r2", "local", "completed", Some(&a), None, "m", 2, (200, 20, 50), 3, 2_000);
    insert(&conn, "r3", "local", "completed", Some(&a), None, "m", 3, (300, 30, 0), 4, 3_000);
    // Not samples: failed, a child, another model, another agent, a plain run, another tenant.
    insert(&conn, "x1", "local", "failed", Some(&a), None, "m", 4, (9, 9, 0), 1, 9);
    insert(&conn, "x2", "local", "completed", Some(&a), Some("r1"), "m", 5, (9, 9, 0), 1, 9);
    insert(&conn, "x3", "local", "completed", Some(&a), None, "other", 6, (9, 9, 0), 1, 9);
    insert(&conn, "x4", "local", "completed", Some(&AgentId::new().to_string()), None, "m", 7, (9, 9, 0), 1, 9);
    insert(&conn, "x5", "local", "completed", None, None, "m", 8, (9, 9, 0), 1, 9);
    insert(&conn, "x6", "other", "completed", Some(&a), None, "m", 9, (9, 9, 0), 1, 9);
    // A compacted run keeps its numbers, so it still counts.
    conn.execute("UPDATE executions SET compacted = 1 WHERE id = 'r1'", []).unwrap();

    let samples = repo.recent_samples(&me, Some(agent), Some("m"), 20).await.unwrap();
    let tokens: Vec<u64> = samples.iter().map(|s| s.input_tokens).collect();
    assert_eq!(tokens, vec![300, 250, 100], "newest first, cached tokens counted as sent");
    assert_eq!(
        (samples[0].output_tokens, samples[0].model_calls, samples[0].active_ms, samples[0].cost_micros),
        (30, 4, 3_000, 600)
    );
    // With no model named, the other model's run counts too.
    assert_eq!(repo.recent_samples(&me, Some(agent), None, 20).await.unwrap().len(), 4);
    assert_eq!(repo.recent_samples(&me, Some(agent), Some("m"), 2).await.unwrap().len(), 2, "a limit applies");
    // A plain run has its own history, and an agent without runs has none.
    assert_eq!(repo.recent_samples(&me, None, Some("m"), 20).await.unwrap().len(), 1);
    assert!(repo.recent_samples(&me, Some(AgentId::new()), Some("m"), 20).await.unwrap().is_empty());
}
