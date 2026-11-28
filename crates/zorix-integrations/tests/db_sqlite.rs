//! The database tools on SQLite: reads cannot change anything or reach other files, writes need a grant and ask first (ADR-0021).

use std::sync::Arc;
use std::time::Duration;

use rusqlite::Connection;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zorix_core::{
    Action, ConnectionKind, ExecutionId, Grants, IntegrationStore, NewConnection, Permission, TenantId, Timestamp,
    Tool, ToolContext, ToolError,
};
use zorix_integrations::{Services, db_tools, secret_name};
use zorix_providers::{MemoryStore, SecretStore};
use zorix_store::SqliteIntegrationStore;

struct Rig {
    dir: tempfile::TempDir,
    store: Arc<SqliteIntegrationStore>,
    tools: Vec<Arc<dyn Tool>>,
}

async fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("app.db");
    let db = Connection::open(&file).unwrap();
    db.execute_batch(
        "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, note TEXT, avatar BLOB);
         INSERT INTO users (name, note, avatar) VALUES ('ana', 'first', x'0102'), ('bo', NULL, NULL), ('cy', 'third', NULL);
         CREATE TABLE big (n INTEGER);
         WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 3000) INSERT INTO big SELECT x FROM c;
         CREATE VIEW names AS SELECT name FROM users;",
    )
    .unwrap();
    drop(db);
    let store = Arc::new(SqliteIntegrationStore::open_in_memory().unwrap());
    let secrets = Arc::new(MemoryStore::new());
    let _ = secret_name; // secrets are not used by SQLite connections
    let _ = secrets.get("x");
    let tools = db_tools(Arc::new(Services { store: store.clone(), secrets }));
    let rig = Rig { dir, store, tools };
    rig.connect("app", file.to_str().unwrap()).await;
    rig
}

impl Rig {
    async fn connect(&self, name: &str, path: &str) {
        self.store
            .add_connection(
                &TenantId::local(),
                NewConnection {
                    kind: ConnectionKind::Sqlite,
                    name: name.into(),
                    settings: json!({"path": path}),
                    has_secret: false,
                },
                Timestamp(1),
            )
            .await
            .unwrap();
    }

    fn file(&self) -> String {
        self.dir.path().join("app.db").to_string_lossy().into_owned()
    }

    async fn call(&self, id: &str, grants: Grants, args: Value) -> Result<Value, ToolError> {
        let ctx = ToolContext {
            execution_id: ExecutionId::new(),
            tenant: TenantId::local(),
            call_id: zorix_core::ToolCallId("c".into()),
            grants,
            cancel: CancellationToken::new(),
            workspace_root: None,
            conversation: None,
            agent: None,
        };
        self.tools.iter().find(|t| t.spec().id == id).unwrap().call(&ctx, args).await
    }
}

fn read() -> Grants {
    Grants::none().with(Permission::new(Action::Read, "db:app"))
}

fn write() -> Grants {
    read().with(Permission::new(Action::Write, "db:app"))
}

fn message(error: ToolError) -> String {
    match error {
        ToolError::Failed(m) | ToolError::Retryable(m) => m,
    }
}

#[tokio::test]
async fn the_schema_lists_tables_views_and_columns() {
    let rig = rig().await;
    let got = rig.call("db.schema", read(), json!({"connection": "app"})).await.unwrap();
    let names: Vec<&str> = got["tables"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["big", "names", "users"]);
    let users = got["tables"].as_array().unwrap().iter().find(|t| t["name"] == "users").unwrap();
    assert_eq!(users["columns"][0], json!({"name": "id", "type": "INTEGER", "nullable": true, "primary_key": true}));
    assert_eq!(users["columns"][1]["nullable"], false);
    let one = rig.call("db.schema", read(), json!({"connection": "app", "table": "names"})).await.unwrap();
    assert_eq!((one["tables"].as_array().unwrap().len(), one["tables"][0]["kind"].as_str()), (1, Some("view")));
}

#[tokio::test]
async fn a_query_returns_rows_nulls_and_a_placeholder_for_binary_data_and_stops_at_the_cap() {
    let rig = rig().await;
    let got = rig
        .call(
            "db.query",
            read(),
            json!({"connection": "app", "sql": "SELECT id, name, note, avatar FROM users ORDER BY id"}),
        )
        .await
        .unwrap();
    assert_eq!(got["columns"], json!(["id", "name", "note", "avatar"]));
    assert_eq!(got["rows"][0], json!([1, "ana", "first", "<2 bytes of binary data>"]));
    assert_eq!(got["rows"][1], json!([2, "bo", null, null]));
    assert_eq!((got["row_count"].as_u64(), got["truncated"].as_bool()), (Some(3), Some(false)));
    let few = rig
        .call("db.query", read(), json!({"connection": "app", "sql": "SELECT n FROM big", "max_rows": 5}))
        .await
        .unwrap();
    assert_eq!((few["row_count"].as_u64(), few["truncated"].as_bool()), (Some(5), Some(true)));
    let many = rig
        .call("db.query", read(), json!({"connection": "app", "sql": "SELECT n FROM big", "max_rows": 1000}))
        .await
        .unwrap();
    assert_eq!(
        (many["row_count"].as_u64(), many["truncated"].as_bool()),
        (Some(1000), Some(true)),
        "never more than 1,000 rows"
    );
}

#[tokio::test]
async fn a_query_cannot_change_anything_attach_another_file_or_change_settings() {
    let rig = rig().await;
    for (sql, why) in [
        ("INSERT INTO users (name) VALUES ('mallory')", "changes data"),
        ("UPDATE users SET name = 'x'", "changes data"),
        ("DELETE FROM users", "changes data"),
        ("DROP TABLE users", "changes data"),
        ("CREATE TABLE t (a)", "changes data"),
        ("ATTACH DATABASE '/etc/passwd' AS other", "not authorized"),
        ("PRAGMA query_only = OFF", "not authorized"),
        ("PRAGMA writable_schema = ON", "not authorized"),
        ("SELECT load_extension('x')", "not authorized"),
    ] {
        let error = message(rig.call("db.query", write(), json!({"connection": "app", "sql": sql})).await.unwrap_err());
        assert!(error.contains(why), "{sql}: {error}");
    }
    // Nothing happened.
    let count =
        rig.call("db.query", read(), json!({"connection": "app", "sql": "SELECT count(*) FROM users"})).await.unwrap();
    assert_eq!(count["rows"][0][0], 3);
    // Reading a table's shape is fine.
    let shape =
        rig.call("db.query", read(), json!({"connection": "app", "sql": "PRAGMA table_info(users)"})).await.unwrap();
    assert_eq!(shape["row_count"], 4);
}

#[tokio::test]
async fn a_statement_that_runs_too_long_is_stopped() {
    let rig = rig().await;
    let started = std::time::Instant::now();
    let error = message(
        zorix_integrations::sqlite_db::query_within(
            &rig.file(),
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT count(*) FROM c",
            10,
            Duration::from_millis(300),
        )
        .unwrap_err(),
    );
    assert!(error.contains("ran longer than") && started.elapsed() < Duration::from_secs(5), "{error}");
}

#[tokio::test]
async fn vacuum_into_cannot_write_a_copy_of_the_database_anywhere() {
    let rig = rig().await;
    let copy = rig.dir.path().join("copy.db");
    let sql = format!("VACUUM INTO '{}'", copy.display());
    // Neither through the tool that changes data (with a write grant), nor through the one that only reads, in any spelling.
    for statement in [
        sql.clone(),
        sql.to_ascii_lowercase(),
        format!("  /* hi */ {sql}"),
        format!("-- x\n{sql}"),
        "VACUUM".to_string(),
    ] {
        for (tool, grants) in [("db.execute", write()), ("db.query", read())] {
            let error =
                message(rig.call(tool, grants, json!({"connection": "app", "sql": statement})).await.unwrap_err());
            assert!(error.contains("VACUUM"), "{tool}: {statement}: {error}");
        }
    }
    assert!(!copy.exists(), "no file was written");
    // A word that only starts with the letters is not the statement.
    let ok = rig.call("db.query", read(), json!({"connection": "app", "sql": "SELECT 'vacuumed'"})).await.unwrap();
    assert_eq!(ok["rows"][0][0], "vacuumed");
}

#[tokio::test]
async fn changing_data_needs_a_write_grant_asks_first_and_runs_one_statement() {
    let rig = rig().await;
    let execute = rig.tools.iter().find(|t| t.spec().id == "db.execute").unwrap();
    assert!(execute.spec().requires_approval && execute.spec().annotations.destructive);
    assert!(!rig.tools.iter().find(|t| t.spec().id == "db.query").unwrap().spec().requires_approval);
    let sql = "INSERT INTO users (name) VALUES ('dee')";
    assert!(
        message(rig.call("db.execute", read(), json!({"connection": "app", "sql": sql})).await.unwrap_err())
            .contains("no permission to write db:app")
    );
    let done = rig.call("db.execute", write(), json!({"connection": "app", "sql": sql})).await.unwrap();
    assert_eq!((done["rows_affected"].as_u64(), done["last_insert_rowid"].as_i64()), (Some(1), Some(4)));
    let updated = rig
        .call("db.execute", write(), json!({"connection": "app", "sql": "UPDATE users SET note = 'x' WHERE id <= 2"}))
        .await
        .unwrap();
    assert_eq!(updated["rows_affected"], 2);
    // Two statements, ATTACH and bad SQL are refused.
    for bad in ["DELETE FROM users; DELETE FROM big", "ATTACH DATABASE ':memory:' AS m", "NOT SQL"] {
        assert!(rig.call("db.execute", write(), json!({"connection": "app", "sql": bad})).await.is_err(), "{bad}");
    }
    let still =
        rig.call("db.query", read(), json!({"connection": "app", "sql": "SELECT count(*) FROM users"})).await.unwrap();
    assert_eq!(still["rows"][0][0], 4, "only the one insert");
    // The card shows the SQL on one line.
    let card =
        execute.describe_call(&json!({"connection": "app", "sql": "DELETE\n  FROM   users\n WHERE id = 1"})).unwrap();
    assert_eq!(card, "Run SQL that changes data in app: DELETE FROM users WHERE id = 1");
}

#[tokio::test]
async fn grants_connections_and_files_are_checked() {
    let rig = rig().await;
    let q = json!({"connection": "app", "sql": "SELECT 1"});
    assert!(message(rig.call("db.query", Grants::none(), q.clone()).await.unwrap_err()).contains("--allow-db app"));
    assert!(
        rig.call("db.query", Grants::none().with(Permission::new(Action::Read, "db:other")), q.clone()).await.is_err()
    );
    assert!(
        rig.call("db.query", Grants::none().with(Permission::new(Action::Read, "db:*")), q.clone()).await.is_ok(),
        "a pattern covers every database"
    );
    let ghost = json!({"connection": "ghost", "sql": "SELECT 1"});
    assert!(
        message(
            rig.call("db.query", Grants::none().with(Permission::new(Action::Read, "db:*")), ghost).await.unwrap_err()
        )
        .contains("no database connection called ghost")
    );
    // A connection whose file is gone, or whose path is relative, says so.
    let gone = std::env::temp_dir().join("zorix-definitely-not-here").join("app.db");
    rig.connect("gone", gone.to_str().unwrap()).await;
    rig.connect("rel", "relative/app.db").await;
    let any = Grants::none().with(Permission::new(Action::Read, "db:*"));
    assert!(
        message(rig.call("db.query", any.clone(), json!({"connection": "gone", "sql": "SELECT 1"})).await.unwrap_err())
            .contains("no SQLite file")
    );
    assert!(
        message(rig.call("db.query", any, json!({"connection": "rel", "sql": "SELECT 1"})).await.unwrap_err())
            .contains("absolute path")
    );
    // Another tenant's connections do not exist here.
    let other = TenantId::new("globex").unwrap();
    assert!(rig.store.connection_named(&other, "app").await.unwrap().is_none());
}
