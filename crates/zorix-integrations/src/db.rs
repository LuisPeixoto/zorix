//! `db.schema`, `db.query` and `db.execute` (ADR-0021).
//!
//! A database is a connection a person set up (SQLite file or Postgres server) and a resource a person grants: `read` or `write`
//! on `db:<connection name>`. A query can only read: SQLite opens the file read-only behind an authorizer, Postgres runs it in a
//! READ ONLY transaction. `db.execute` changes data, takes a write grant and always asks, and the approval card shows the SQL.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use zorix_core::{
    Action, ConnectionKind, Permission, Tool, ToolAnnotations, ToolContext, ToolError, ToolSource, ToolSpec,
};

use crate::connections::{Services, clip};
use crate::{postgres_db, sqlite_db};

/// The most rows of one answer.
pub const MAX_ROWS: usize = 1_000;
/// The most bytes of cell text of one answer.
pub const MAX_BYTES: usize = 5 * 1024 * 1024;
/// How long one statement may run.
pub const STATEMENT_TIMEOUT: Duration = Duration::from_secs(15);

fn failed(message: impl Into<String>) -> ToolError {
    ToolError::Failed(message.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Schema,
    Query,
    Execute,
}

impl Op {
    fn id(self) -> &'static str {
        match self {
            Op::Schema => "schema",
            Op::Query => "query",
            Op::Execute => "execute",
        }
    }
}

/// A database tool.
pub struct DbTool {
    services: Arc<Services>,
    op: Op,
    spec: ToolSpec,
}

/// The database tools.
pub fn db_tools(services: Arc<Services>) -> Vec<Arc<dyn Tool>> {
    let connection = json!({"type": "string", "minLength": 1, "maxLength": 32, "description": "The name of the database connection."});
    [Op::Schema, Op::Query, Op::Execute]
        .into_iter()
        .map(|op| {
            let (describe, props, required): (&str, Vec<(&str, Value)>, Vec<&str>) = match op {
                Op::Schema => (
                    "List the tables and columns of a database.",
                    vec![("table", json!({"type": "string", "maxLength": 128, "description": "Only this table."}))],
                    vec!["connection"],
                ),
                Op::Query => (
                    "Run one read-only SQL statement (SELECT, WITH, VALUES, TABLE, SHOW or EXPLAIN) and return its rows, up to 1,000. It cannot change anything.",
                    vec![
                        ("sql", json!({"type": "string", "minLength": 1, "maxLength": 20000})),
                        ("max_rows", json!({"type": "integer", "minimum": 1, "maximum": MAX_ROWS, "description": "How many rows at most (default 100)."})),
                    ],
                    vec!["connection", "sql"],
                ),
                Op::Execute => (
                    "Run one SQL statement that changes data (INSERT, UPDATE, DELETE, DDL). A person reads the SQL and approves it first.",
                    vec![("sql", json!({"type": "string", "minLength": 1, "maxLength": 20000}))],
                    vec!["connection", "sql"],
                ),
            };
            let mut properties: serde_json::Map<String, Value> = props.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
            properties.insert("connection".into(), connection.clone());
            let mut spec = ToolSpec::new(
                format!("db.{}", op.id()),
                format!("Database {}", op.id()),
                describe.to_string(),
                json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false}),
            );
            spec.source = ToolSource::Integration { connection: None };
            spec.timeout_ms = 45_000;
            spec.max_output_bytes = 256 * 1024;
            spec.requires_approval = op == Op::Execute;
            spec.annotations = ToolAnnotations { read_only: op != Op::Execute, destructive: op == Op::Execute, idempotent: op != Op::Execute, open_world: false };
            Arc::new(DbTool { services: services.clone(), op, spec }) as Arc<dyn Tool>
        })
        .collect()
}

impl DbTool {
    async fn run(&self, ctx: &ToolContext, args: &Value) -> Result<Value, ToolError> {
        let name = args
            .get("connection")
            .and_then(Value::as_str)
            .filter(|n| !n.is_empty())
            .ok_or_else(|| failed("connection is missing."))?;
        let action = if self.op == Op::Execute { Action::Write } else { Action::Read };
        let needed = Permission::new(action, format!("db:{name}"));
        if !ctx.grants.allows(&needed) {
            return Err(failed(format!(
                "This execution has no permission to {needed}. A person can grant it with `--allow-db {name}{}`.",
                if action == Action::Write { ":write" } else { "" }
            )));
        }
        let connection = self
            .services
            .store
            .connection_named(&ctx.tenant, name)
            .await
            .map_err(|e| failed(e.to_string()))?
            .filter(|c| matches!(c.kind, ConnectionKind::Sqlite | ConnectionKind::Postgres))
            .ok_or_else(|| failed(format!("There is no database connection called {name}.")))?;
        let sql = args.get("sql").and_then(Value::as_str).unwrap_or("").trim().to_string();
        if self.op != Op::Schema && sql.is_empty() {
            return Err(failed("sql is missing."));
        }
        let max_rows =
            args.get("max_rows").and_then(Value::as_u64).map(|n| (n as usize).clamp(1, MAX_ROWS)).unwrap_or(100);
        let table = args.get("table").and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_string);
        match connection.kind {
            ConnectionKind::Sqlite => {
                let path = connection.settings["path"]
                    .as_str()
                    .ok_or_else(|| failed("The SQLite connection has no file path."))?
                    .to_string();
                let op = self.op;
                tokio::task::spawn_blocking(move || match op {
                    Op::Schema => sqlite_db::schema(&path, table.as_deref()),
                    Op::Query => sqlite_db::query(&path, &sql, max_rows),
                    Op::Execute => sqlite_db::execute(&path, &sql),
                })
                .await
                .map_err(|e| failed(format!("The database call failed: {e}")))?
            }
            _ => {
                let password = self.services.secret(&connection).await?;
                match self.op {
                    Op::Schema => postgres_db::schema(&connection, password.as_deref(), table.as_deref()).await,
                    Op::Query => postgres_db::query(&connection, password.as_deref(), &sql, max_rows).await,
                    Op::Execute => postgres_db::execute(&connection, password.as_deref(), &sql).await,
                }
            }
        }
    }
}

#[async_trait]
impl Tool for DbTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        self.run(ctx, &args).await
    }

    fn describe_call(&self, args: &Value) -> Option<String> {
        let name = args.get("connection").and_then(Value::as_str)?;
        Some(match self.op {
            Op::Execute => format!(
                "Run SQL that changes data in {name}: {}",
                clip(
                    &args
                        .get("sql")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" "),
                    160
                )
            ),
            Op::Query => format!(
                "Read from {name}: {}",
                clip(
                    &args
                        .get("sql")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" "),
                    160
                )
            ),
            Op::Schema => format!("List the tables of {name}"),
        })
    }
}
