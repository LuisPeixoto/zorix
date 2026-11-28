//! SQLite access for `db.*` (ADR-0021). A query opens the file read-only and is checked by SQLite's authorizer, so it cannot
//! attach another file, load an extension or change settings; it is interrupted after a time limit and stops at the row and size
//! caps. `db.execute` opens it for writing, with the same authorizer, and runs one statement.

use std::path::Path;
use std::time::{Duration, Instant};

use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use zorix_core::ToolError;

use crate::db::{MAX_BYTES, MAX_ROWS, STATEMENT_TIMEOUT};

fn failed(message: impl Into<String>) -> ToolError {
    ToolError::Failed(message.into())
}

/// PRAGMAs that only report. Anything else could change how the connection behaves.
const SAFE_PRAGMAS: &[&str] = &[
    "table_info",
    "table_xinfo",
    "table_list",
    "index_list",
    "index_info",
    "index_xinfo",
    "foreign_key_list",
    "database_list",
    "collation_list",
    "function_list",
    "schema_version",
    "user_version",
    "page_count",
    "page_size",
    "encoding",
    "journal_mode",
];

fn authorize(ctx: AuthContext<'_>) -> Authorization {
    match ctx.action {
        AuthAction::Attach { .. } | AuthAction::Detach { .. } => Authorization::Deny,
        AuthAction::Pragma { pragma_name, pragma_value } => {
            if SAFE_PRAGMAS.contains(&pragma_name.to_ascii_lowercase().as_str())
                && (pragma_value.is_none() || pragma_name.eq_ignore_ascii_case("table_info"))
            {
                Authorization::Allow
            } else {
                Authorization::Deny
            }
        }
        AuthAction::Function { function_name } if function_name.eq_ignore_ascii_case("load_extension") => {
            Authorization::Deny
        }
        _ => Authorization::Allow,
    }
}

fn open(path: &str, write: bool) -> Result<Connection, ToolError> {
    let file = Path::new(path);
    if !file.is_absolute() {
        return Err(failed("The SQLite connection's file must be an absolute path."));
    }
    if !file.is_file() {
        return Err(failed(format!("There is no SQLite file at {path}.")));
    }
    let flags = if write { OpenFlags::SQLITE_OPEN_READ_WRITE } else { OpenFlags::SQLITE_OPEN_READ_ONLY }
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let mut conn = Connection::open_with_flags(file, flags)
        .map_err(|e| failed(format!("The SQLite file could not be opened: {e}")))?;
    conn.busy_timeout(Duration::from_secs(5)).map_err(|e| failed(e.to_string()))?;
    // If the guard cannot be installed the file is not opened at all.
    conn.authorizer(Some(authorize))
        .map_err(|e| failed(format!("The SQLite safety checks could not be installed: {e}")))?;
    Ok(conn)
}

fn value(v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => json!(i),
        ValueRef::Real(f) => json!(f),
        ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
        ValueRef::Blob(b) => json!(format!("<{} bytes of binary data>", b.len())),
    }
}

/// Run a read-only statement and return its rows.
pub fn query(path: &str, sql: &str, max_rows: usize) -> Result<Value, ToolError> {
    query_within(path, sql, max_rows, STATEMENT_TIMEOUT)
}

/// The same with a chosen time limit (for tests).
pub fn query_within(path: &str, sql: &str, max_rows: usize, limit: Duration) -> Result<Value, ToolError> {
    let mut conn = open(path, false)?;
    let deadline = Instant::now() + limit;
    conn.progress_handler(1000, Some(move || Instant::now() > deadline)).map_err(|e| failed(e.to_string()))?;
    if is_vacuum(sql) {
        return Err(failed("VACUUM is not a query."));
    }
    let mut statement = conn.prepare(sql).map_err(|e| failed(format!("SQLite rejected the statement: {e}")))?;
    if !statement.readonly() {
        return Err(failed("That statement changes data. Use db.execute for it."));
    }
    let columns: Vec<String> = statement.column_names().map(str::to_string).collect();
    let mut rows = statement.query([]).map_err(|e| failed(format!("The query failed: {e}")))?;
    let mut out: Vec<Value> = Vec::new();
    let mut bytes = 0usize;
    let mut truncated = false;
    loop {
        match rows.next() {
            Ok(Some(row)) => {
                if out.len() >= max_rows {
                    truncated = true;
                    break;
                }
                let cells: Vec<Value> =
                    (0..columns.len()).map(|i| row.get_ref(i).map(value).unwrap_or(Value::Null)).collect();
                bytes += cells.iter().map(|c| c.to_string().len()).sum::<usize>();
                if bytes > MAX_BYTES {
                    truncated = true;
                    break;
                }
                out.push(Value::Array(cells));
            }
            Ok(None) => break,
            Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::OperationInterrupted => {
                return Err(failed(format!(
                    "The query ran longer than {} seconds and was stopped.",
                    STATEMENT_TIMEOUT.as_secs()
                )));
            }
            Err(e) => return Err(failed(format!("The query failed: {e}"))),
        }
    }
    Ok(json!({"columns": columns, "rows": out, "row_count": out.len(), "truncated": truncated}))
}

/// Whether a statement starts with `VACUUM`, skipping spaces and comments. `VACUUM INTO 'path'` writes a copy of the database to any
/// place the program can write, which no authorizer action covers.
fn is_vacuum(sql: &str) -> bool {
    let mut rest = sql.trim_start();
    loop {
        if let Some(after) = rest.strip_prefix("--") {
            rest = after.split_once('\n').map(|(_, r)| r).unwrap_or("").trim_start();
        } else if let Some(after) = rest.strip_prefix("/*") {
            rest = after.split_once("*/").map(|(_, r)| r).unwrap_or("").trim_start();
        } else {
            break;
        }
    }
    rest.get(..6).is_some_and(|w| w.eq_ignore_ascii_case("vacuum"))
        && rest[6..].chars().next().is_none_or(|c| !c.is_ascii_alphanumeric() && c != '_')
}

/// Run one statement that changes data.
pub fn execute(path: &str, sql: &str) -> Result<Value, ToolError> {
    if is_vacuum(sql) {
        return Err(failed(
            "VACUUM is not run through db.execute: VACUUM INTO writes a copy of the database wherever it is told, and a plain VACUUM changes no data.",
        ));
    }
    let mut conn = open(path, true)?;
    let deadline = Instant::now() + STATEMENT_TIMEOUT;
    conn.progress_handler(1000, Some(move || Instant::now() > deadline)).map_err(|e| failed(e.to_string()))?;
    let mut statement = conn.prepare(sql).map_err(|e| failed(format!("SQLite rejected the statement: {e}")))?;
    let affected = statement.execute([]).map_err(|e| failed(format!("The statement failed: {e}")))?;
    Ok(json!({"rows_affected": affected, "last_insert_rowid": conn.last_insert_rowid()}))
}

/// The tables and their columns.
pub fn schema(path: &str, table: Option<&str>) -> Result<Value, ToolError> {
    let conn = open(path, false)?;
    let mut tables = Vec::new();
    let mut statement = conn
        .prepare("SELECT name, type FROM sqlite_master WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .map_err(|e| failed(e.to_string()))?;
    let names: Vec<(String, String)> = statement
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| failed(e.to_string()))?
        .filter_map(Result::ok)
        .collect();
    for (name, kind) in names.into_iter().filter(|(n, _)| table.is_none_or(|t| t == n)).take(MAX_ROWS) {
        let mut info = conn
            .prepare(&format!("PRAGMA table_info(\"{}\")", name.replace('"', "\"\"")))
            .map_err(|e| failed(e.to_string()))?;
        let columns: Vec<Value> = info
            .query_map([], |r| Ok(json!({"name": r.get::<_, String>(1)?, "type": r.get::<_, String>(2)?, "nullable": r.get::<_, i64>(3)? == 0, "primary_key": r.get::<_, i64>(5)? > 0})))
            .map_err(|e| failed(e.to_string()))?
            .filter_map(Result::ok)
            .collect();
        tables.push(json!({"name": name, "kind": kind, "columns": columns}));
    }
    Ok(json!({"tables": tables}))
}
