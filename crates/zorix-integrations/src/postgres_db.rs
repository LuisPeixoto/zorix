//! Postgres access for `db.*` (ADR-0021). TLS is on (the system's trusted roots, plus a CA file when the connection names one);
//! `disable` is only for this computer. A query runs inside a READ ONLY transaction after a first statement has fixed that mode
//! (so the statement itself cannot switch it), with a statement timeout; `db.execute` runs one statement in its own transaction.

use std::time::Duration;

use rustls::ClientConfig;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;
use rustls_platform_verifier::ConfigVerifierExt;
use serde_json::{Value, json};
use tokio_postgres::{Client, Config, NoTls};
use tokio_postgres_rustls_improved::MakeRustlsConnect;
use zorix_core::{Connection, ToolError, redact_text};
use zorix_providers::is_loopback;

use crate::db::{MAX_BYTES, STATEMENT_TIMEOUT};

fn failed(message: impl Into<String>) -> ToolError {
    ToolError::Failed(message.into())
}

fn tls_config(ca_file: Option<&str>) -> Result<ClientConfig, ToolError> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    if let Some(path) = ca_file {
        let mut roots = rustls::RootCertStore::empty();
        let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(path)
            .map_err(|e| failed(format!("The CA file {path} could not be read: {e}")))?
            .collect::<Result<_, _>>()
            .map_err(|e| failed(format!("The CA file {path} is not valid: {e}")))?;
        let (added, _) = roots.add_parsable_certificates(certs);
        if added == 0 {
            return Err(failed(format!("The CA file {path} holds no certificate.")));
        }
        return Ok(ClientConfig::builder().with_root_certificates(roots).with_no_client_auth());
    }
    ClientConfig::with_platform_verifier()
        .map_err(|e| failed(format!("The system's certificate store could not be used: {e}")))
}

/// A client connected to the database a connection describes.
async fn connect(connection: &Connection, password: Option<&str>) -> Result<Client, ToolError> {
    let s = &connection.settings;
    let host = s["host"].as_str().ok_or_else(|| failed("The Postgres connection has no host."))?;
    let mut config = Config::new();
    config
        .host(host)
        .port(s["port"].as_u64().unwrap_or(5432) as u16)
        .dbname(s["database"].as_str().ok_or_else(|| failed("The Postgres connection has no database."))?)
        .user(s["user"].as_str().ok_or_else(|| failed("The Postgres connection has no user."))?)
        .connect_timeout(Duration::from_secs(10))
        .application_name("zorix");
    if let Some(password) = password {
        config.password(password);
    }
    let mode = s["sslmode"].as_str().unwrap_or("verify-full");
    let client = match mode {
        "disable" if is_loopback(host) => {
            let (client, connection) = config
                .connect(NoTls)
                .await
                .map_err(|e| failed(format!("Postgres could not be reached: {}", redact_text(&e.to_string()))))?;
            tokio::spawn(async move {
                let _ = connection.await;
            });
            client
        }
        "disable" => return Err(failed("Encryption can only be switched off for a database on this computer.")),
        "verify-full" => {
            let tls = MakeRustlsConnect::new(tls_config(s["ca_file"].as_str())?);
            let (client, connection) = config
                .connect(tls)
                .await
                .map_err(|e| failed(format!("Postgres could not be reached: {}", redact_text(&e.to_string()))))?;
            tokio::spawn(async move {
                let _ = connection.await;
            });
            client
        }
        other => {
            return Err(failed(format!(
                "The sslmode “{other}” is not supported; use verify-full (or disable, for this computer)."
            )));
        }
    };
    Ok(client)
}

fn db_error(e: tokio_postgres::Error) -> ToolError {
    match e.as_db_error() {
        Some(db) => failed(format!("Postgres said: {}", redact_text(db.message()))),
        None => failed(format!("The statement failed: {}", redact_text(&e.to_string()))),
    }
}

/// The first word of a statement, in lowercase, skipping spaces and comments.
pub fn first_word(sql: &str) -> String {
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
    rest.split(|c: char| !c.is_ascii_alphabetic()).next().unwrap_or("").to_ascii_lowercase()
}

/// `sql` without one trailing semicolon. Anything else that looks like a second statement is refused by the server.
fn single(sql: &str) -> &str {
    sql.trim().trim_end_matches(';').trim()
}

/// Run a read-only statement.
pub async fn query(
    connection: &Connection,
    password: Option<&str>,
    sql: &str,
    max_rows: usize,
) -> Result<Value, ToolError> {
    let word = first_word(sql);
    if !matches!(word.as_str(), "select" | "with" | "values" | "table" | "show" | "explain") {
        return Err(failed(
            "db.query runs SELECT, WITH, VALUES, TABLE, SHOW and EXPLAIN. Use db.execute for statements that change data.",
        ));
    }
    let client = connect(connection, password).await?;
    let work = async {
        client
            .batch_execute(&format!(
                "BEGIN READ ONLY; SET LOCAL statement_timeout = {}; SET LOCAL lock_timeout = 3000; SELECT 1",
                STATEMENT_TIMEOUT.as_millis()
            ))
            .await
            .map_err(db_error)?;
        let statement = single(sql);
        let result = if matches!(word.as_str(), "show" | "explain") {
            let rows = client.query(statement, &[]).await.map_err(db_error)?;
            let lines: Vec<Value> =
                rows.iter().map(|r| json!([r.try_get::<_, String>(0).unwrap_or_default()])).collect();
            json!({"columns": ["result"], "rows": lines, "row_count": lines.len(), "truncated": false})
        } else {
            // The statement gets its own lines, so a `--` comment at its end cannot swallow the closing parenthesis.
            let wrapped = format!("SELECT to_jsonb(q) FROM (\n{statement}\n) AS q LIMIT {}", max_rows + 1);
            let rows = client.query(&wrapped, &[]).await.map_err(db_error)?;
            let mut columns: Vec<String> = Vec::new();
            let mut out: Vec<Value> = Vec::new();
            let mut bytes = 0usize;
            let mut truncated = false;
            for row in &rows {
                let object: Value = row.try_get(0).map_err(|e| failed(format!("A row could not be read: {e}")))?;
                if out.len() >= max_rows {
                    truncated = true;
                    break;
                }
                let map = object.as_object().cloned().unwrap_or_default();
                if columns.is_empty() {
                    columns = map.keys().cloned().collect();
                }
                let cells: Vec<Value> = columns.iter().map(|c| map.get(c).cloned().unwrap_or(Value::Null)).collect();
                bytes += cells.iter().map(|c| c.to_string().len()).sum::<usize>();
                if bytes > MAX_BYTES {
                    truncated = true;
                    break;
                }
                out.push(Value::Array(cells));
            }
            json!({"columns": columns, "rows": out, "row_count": out.len(), "truncated": truncated})
        };
        let _ = client.batch_execute("ROLLBACK").await;
        Ok::<_, ToolError>(result)
    };
    tokio::time::timeout(STATEMENT_TIMEOUT + Duration::from_secs(10), work).await.map_err(|_| {
        failed(format!("The query ran longer than {} seconds and was stopped.", STATEMENT_TIMEOUT.as_secs()))
    })?
}

/// Run one statement that changes data, in its own transaction.
pub async fn execute(connection: &Connection, password: Option<&str>, sql: &str) -> Result<Value, ToolError> {
    let client = connect(connection, password).await?;
    let work = async {
        client
            .batch_execute(&format!(
                "BEGIN; SET LOCAL statement_timeout = {}; SET LOCAL lock_timeout = 3000",
                STATEMENT_TIMEOUT.as_millis()
            ))
            .await
            .map_err(db_error)?;
        match client.execute(single(sql), &[]).await {
            Ok(affected) => {
                client.batch_execute("COMMIT").await.map_err(db_error)?;
                Ok(json!({"rows_affected": affected}))
            }
            Err(e) => {
                let _ = client.batch_execute("ROLLBACK").await;
                Err(db_error(e))
            }
        }
    };
    tokio::time::timeout(STATEMENT_TIMEOUT + Duration::from_secs(10), work).await.map_err(|_| {
        failed(format!("The statement ran longer than {} seconds and was stopped.", STATEMENT_TIMEOUT.as_secs()))
    })?
}

/// The tables and columns the user can see, outside the system schemas.
pub async fn schema(connection: &Connection, password: Option<&str>, table: Option<&str>) -> Result<Value, ToolError> {
    let client = connect(connection, password).await?;
    let rows = client
        .query(
            "SELECT table_schema, table_name, column_name, data_type, is_nullable FROM information_schema.columns \
             WHERE table_schema NOT IN ('pg_catalog', 'information_schema') AND ($1::text IS NULL OR table_name = $1) \
             ORDER BY table_schema, table_name, ordinal_position LIMIT 3000",
            &[&table],
        )
        .await
        .map_err(db_error)?;
    let mut tables: Vec<Value> = Vec::new();
    let mut current = String::new();
    for row in &rows {
        let (schema, name): (String, String) = (row.get(0), row.get(1));
        let full = format!("{schema}.{name}");
        if full != current {
            tables.push(json!({"name": full, "columns": []}));
            current = full;
        }
        let column = json!({"name": row.get::<_, String>(2), "type": row.get::<_, String>(3), "nullable": row.get::<_, String>(4) == "YES"});
        if let Some(last) = tables.last_mut() {
            last["columns"].as_array_mut().expect("columns is an array").push(column);
        }
    }
    Ok(json!({"tables": tables, "truncated": rows.len() >= 3000}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_word_ignores_spaces_and_comments() {
        assert_eq!(first_word("  SELECT 1"), "select");
        assert_eq!(first_word("-- hi\n/* a */ with x as (select 1) select * from x"), "with");
        assert_eq!(first_word("/* unfinished"), "");
        assert_eq!(first_word("DROP TABLE x"), "drop");
        assert_eq!(first_word(""), "");
    }

    #[test]
    fn one_trailing_semicolon_is_dropped() {
        assert_eq!(single("select 1;"), "select 1");
        assert_eq!(single("  select 1 ;; "), "select 1");
        assert_eq!(single("select 'a;b'"), "select 'a;b'");
    }
}
