//! The database tools on Postgres (ADR-0021).
//!
//! These need a server. Set `ZORIX_TEST_POSTGRES=host:port:user:password:database` to run them (CI does, with a service
//! container); without it they do nothing and pass. The password is never printed.

use std::sync::Arc;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zorix_core::{
    Action, ConnectionKind, ExecutionId, Grants, IntegrationStore, NewConnection, Permission, Secret, TenantId,
    Timestamp, Tool, ToolContext, ToolError,
};
use zorix_integrations::{Services, db_tools, secret_name};
use zorix_providers::{MemoryStore, SecretStore};
use zorix_store::SqliteIntegrationStore;

struct Rig {
    tools: Vec<Arc<dyn Tool>>,
    table: String,
}

fn settings() -> Option<(Value, String)> {
    let text = std::env::var("ZORIX_TEST_POSTGRES").ok()?;
    let parts: Vec<&str> = text.splitn(5, ':').collect();
    if parts.len() != 5 {
        panic!("ZORIX_TEST_POSTGRES must be host:port:user:password:database");
    }
    Some((
        json!({"host": parts[0], "port": parts[1].parse::<u64>().unwrap(), "user": parts[2], "database": parts[4], "sslmode": "disable"}),
        parts[3].to_string(),
    ))
}

async fn rig(name: &str, settings: Value, password: &str) -> Rig {
    let store = Arc::new(SqliteIntegrationStore::open_in_memory().unwrap());
    let secrets = Arc::new(MemoryStore::new());
    let connection = store
        .add_connection(
            &TenantId::local(),
            NewConnection { kind: ConnectionKind::Postgres, name: name.into(), settings, has_secret: true },
            Timestamp(1),
        )
        .await
        .unwrap();
    secrets.set(&secret_name(connection.id), &Secret::new(password)).unwrap();
    let tools = db_tools(Arc::new(Services { store, secrets, signins: Default::default() }));
    Rig { tools, table: format!("zorix_t_{}", std::process::id()) }
}

impl Rig {
    async fn call(&self, id: &str, args: Value) -> Result<Value, ToolError> {
        let grants =
            Grants::none().with(Permission::new(Action::Read, "db:*")).with(Permission::new(Action::Write, "db:*"));
        let ctx = ToolContext {
            execution_id: ExecutionId::new(),
            tenant: TenantId::local(),
            call_id: zorix_core::ToolCallId("c".into()),
            grants,
            cancel: CancellationToken::new(),
            workspace_root: None,
            conversation: None,
            agent: None,
            host: None,
        };
        self.tools.iter().find(|t| t.spec().id == id).unwrap().call(&ctx, args).await
    }

    async fn sql(&self, id: &str, sql: &str) -> Result<Value, ToolError> {
        self.call(id, json!({"connection": "pg", "sql": sql})).await
    }
}

fn message(error: ToolError) -> String {
    match error {
        ToolError::Failed(m) | ToolError::Retryable(m) => m,
    }
}

#[tokio::test]
async fn rows_come_back_as_json_whatever_their_types_and_reads_cannot_write() {
    let Some((settings, password)) = settings() else { return };
    let rig = rig("pg", settings, &password).await;
    let t = &rig.table;
    rig.sql("db.execute", &format!("DROP TABLE IF EXISTS {t}")).await.unwrap();
    rig.sql("db.execute", &format!("CREATE TABLE {t} (id serial PRIMARY KEY, name text NOT NULL, price numeric(10,2), at timestamptz, meta jsonb, ok boolean)")).await.unwrap();
    let inserted = rig.sql("db.execute", &format!("INSERT INTO {t} (name, price, at, meta, ok) VALUES ('ana', 12.50, '2026-11-19T10:00:00Z', '{{\"a\": [1, 2]}}', true), ('bo', NULL, NULL, NULL, false)")).await.unwrap();
    assert_eq!(inserted["rows_affected"], 2);

    let got = rig
        .sql("db.query", &format!("SELECT id, name, price, at, meta, ok FROM {t} ORDER BY id -- newest last"))
        .await
        .unwrap();
    assert_eq!(got["row_count"], 2);
    let columns: Vec<String> =
        got["columns"].as_array().unwrap().iter().map(|c| c.as_str().unwrap().to_string()).collect();
    let cell = |row: usize, name: &str| got["rows"][row][columns.iter().position(|c| c == name).unwrap()].clone();
    assert_eq!((cell(0, "name"), cell(0, "ok"), cell(0, "meta")), (json!("ana"), json!(true), json!({"a": [1, 2]})));
    assert_eq!(cell(0, "price"), json!(12.50), "numeric comes as a JSON number");
    assert!(cell(0, "at").as_str().unwrap().starts_with("2026-11-19T10:00:00"));
    assert_eq!((cell(1, "price"), cell(1, "meta")), (Value::Null, Value::Null));

    // The schema shows the table and its columns.
    let schema = rig.call("db.schema", json!({"connection": "pg", "table": t})).await.unwrap();
    assert_eq!(schema["tables"][0]["name"], format!("public.{t}"));
    assert_eq!(schema["tables"][0]["columns"].as_array().unwrap().len(), 6);

    // A query cannot change anything, whichever way it is written.
    for sql in [
        format!("DELETE FROM {t}"),
        format!("DROP TABLE {t}"),
        "SET default_transaction_read_only = off".into(),
        "COMMIT".into(),
    ] {
        assert!(message(rig.sql("db.query", &sql).await.unwrap_err()).contains("db.execute"), "{sql}");
    }
    for sql in [
        format!("WITH gone AS (DELETE FROM {t} RETURNING *) SELECT * FROM gone"),
        format!("WITH made AS (INSERT INTO {t} (name) VALUES ('mallory') RETURNING *) SELECT * FROM made"),
        "SELECT set_config('transaction_read_only', 'off', true)".into(),
    ] {
        let error = message(rig.sql("db.query", &sql).await.unwrap_err());
        assert!(
            error.contains("read-only") || error.contains("read-write") || error.contains("data-modifying"),
            "{sql}: {error}"
        );
    }
    let still = rig.sql("db.query", &format!("SELECT count(*) AS n FROM {t}")).await.unwrap();
    assert_eq!(still["rows"][0][0], 2, "nothing was changed");

    // SHOW and EXPLAIN work; the row cap holds.
    assert_eq!(rig.sql("db.query", "SHOW server_version").await.unwrap()["row_count"], 1);
    assert!(rig.sql("db.query", "EXPLAIN SELECT 1").await.unwrap()["row_count"].as_u64().unwrap() >= 1);
    let many = rig
        .call("db.query", json!({"connection": "pg", "sql": "SELECT generate_series(1, 5000) AS n", "max_rows": 1000}))
        .await
        .unwrap();
    assert_eq!((many["row_count"].as_u64(), many["truncated"].as_bool()), (Some(1000), Some(true)));
    rig.sql("db.execute", &format!("DROP TABLE {t}")).await.unwrap();
}

#[tokio::test]
async fn a_statement_that_fails_leaves_nothing_behind_and_only_one_statement_runs() {
    let Some((settings, password)) = settings() else { return };
    let rig = rig("pg", settings, &password).await;
    let t = format!("{}_b", rig.table);
    rig.sql("db.execute", &format!("DROP TABLE IF EXISTS {t}")).await.unwrap();
    rig.sql("db.execute", &format!("CREATE TABLE {t} (id int PRIMARY KEY)")).await.unwrap();
    // The second row breaks the primary key: the whole statement is undone.
    let error = message(rig.sql("db.execute", &format!("INSERT INTO {t} VALUES (1), (1)")).await.unwrap_err());
    assert!(error.contains("Postgres said") && error.contains("duplicate"), "{error}");
    assert_eq!(rig.sql("db.query", &format!("SELECT count(*) FROM {t}")).await.unwrap()["rows"][0][0], 0);
    // Two statements are refused by the server, and neither runs.
    assert!(rig.sql("db.execute", &format!("INSERT INTO {t} VALUES (7); INSERT INTO {t} VALUES (8)")).await.is_err());
    assert_eq!(rig.sql("db.query", &format!("SELECT count(*) FROM {t}")).await.unwrap()["rows"][0][0], 0);
    let ok = rig.sql("db.execute", &format!("UPDATE {t} SET id = id")).await.unwrap();
    assert_eq!(ok["rows_affected"], 0);
    rig.sql("db.execute", &format!("DROP TABLE {t}")).await.unwrap();
}

#[tokio::test]
async fn a_slow_statement_is_stopped_and_a_wrong_password_is_reported_without_it() {
    let Some((settings, password)) = settings() else { return };
    let rig = self::rig("pg", settings.clone(), &password).await;
    let error = message(rig.sql("db.query", "SELECT pg_sleep(60)").await.unwrap_err());
    assert!(error.contains("statement timeout") || error.contains("ran longer"), "{error}");
    let wrong = self::rig("pg", settings, "definitely-not-the-password").await;
    let error = message(wrong.sql("db.query", "SELECT 1").await.unwrap_err());
    assert!(error.contains("could not be reached") && !error.contains("definitely-not-the-password"), "{error}");
}

#[tokio::test]
async fn encryption_cannot_be_switched_off_for_another_computer_and_a_server_without_tls_is_not_trusted() {
    // No server needed: the refusal comes before anything is sent.
    let rig =
        self::rig("pg", json!({"host": "db.example.com", "user": "u", "database": "d", "sslmode": "disable"}), "p")
            .await;
    assert!(
        message(rig.sql("db.query", "SELECT 1").await.unwrap_err())
            .contains("only be switched off for a database on this computer")
    );
    let rig =
        self::rig("pg", json!({"host": "127.0.0.1", "user": "u", "database": "d", "sslmode": "prefer"}), "p").await;
    assert!(message(rig.sql("db.query", "SELECT 1").await.unwrap_err()).contains("sslmode"));
    let rig = self::rig("pg", json!({"host": "127.0.0.1", "user": "u"}), "p").await;
    assert!(message(rig.sql("db.query", "SELECT 1").await.unwrap_err()).contains("no database"));
    // With a real server and the default (verify-full), a server without TLS is refused.
    if let Some((mut settings, password)) = settings() {
        settings.as_object_mut().unwrap().remove("sslmode");
        let rig = self::rig("pg", settings, &password).await;
        let error = message(rig.sql("db.query", "SELECT 1").await.unwrap_err());
        assert!(error.contains("could not be reached"), "{error}");
    }
}

#[tokio::test]
async fn an_answer_is_read_as_it_arrives_and_cut_at_the_limits() {
    let Some((settings, password)) = settings() else { return };
    let rig = rig("pg", settings, &password).await;
    // Forty rows of one megabyte: the byte limit stops the reading after a few, without all forty being collected first.
    let big = rig
        .call(
            "db.query",
            json!({"connection": "pg", "sql": "SELECT repeat('x', 1000000) AS big FROM generate_series(1, 40)", "max_rows": 1000}),
        )
        .await
        .unwrap();
    assert_eq!(big["truncated"], true);
    assert!(big["row_count"].as_u64().unwrap() <= 6, "{}", big["row_count"]);
    // SHOW ALL has hundreds of rows: the row limit holds for it too.
    let all = rig.call("db.query", json!({"connection": "pg", "sql": "SHOW ALL", "max_rows": 10})).await.unwrap();
    assert_eq!((all["row_count"].as_u64(), all["truncated"].as_bool()), (Some(10), Some(true)));
    // What was not wanted did not wedge anything: the next query on a new connection is fine.
    assert_eq!(rig.sql("db.query", "SELECT 1 AS n").await.unwrap()["rows"][0][0], 1);
}
