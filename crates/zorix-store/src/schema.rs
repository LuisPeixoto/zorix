//! The database schema, as numbered migrations. `PRAGMA user_version` holds the number applied so far.

use rusqlite::Connection;

/// Migrations in order. Never edit one that has shipped: add a new one.
const MIGRATIONS: &[&str] = &[
    // 1: the first schema. Whole records are stored as JSON next to the few columns that are queried.
    "
    CREATE TABLE executions (
        id TEXT PRIMARY KEY,
        tenant TEXT NOT NULL,
        conversation TEXT,
        state TEXT NOT NULL,
        version INTEGER NOT NULL,
        idempotency_key TEXT,
        created_at INTEGER NOT NULL,
        doc TEXT NOT NULL
    );
    CREATE UNIQUE INDEX executions_idempotency ON executions (tenant, idempotency_key) WHERE idempotency_key IS NOT NULL;
    CREATE INDEX executions_by_tenant ON executions (tenant, created_at DESC, id DESC);
    CREATE INDEX executions_by_state ON executions (state);

    CREATE TABLE events (
        seq INTEGER PRIMARY KEY AUTOINCREMENT,
        execution_id TEXT NOT NULL,
        at INTEGER NOT NULL,
        schema_version INTEGER NOT NULL,
        kind TEXT NOT NULL,
        payload TEXT NOT NULL
    );
    CREATE INDEX events_by_execution ON events (execution_id, seq);

    CREATE TABLE checkpoints (
        execution_id TEXT PRIMARY KEY,
        step INTEGER NOT NULL,
        created_at INTEGER NOT NULL,
        doc TEXT NOT NULL
    );

    CREATE TABLE tool_calls (
        execution_id TEXT NOT NULL,
        call_id TEXT NOT NULL,
        ord INTEGER NOT NULL,
        doc TEXT NOT NULL,
        PRIMARY KEY (execution_id, call_id)
    );

    CREATE TABLE approvals (
        id TEXT PRIMARY KEY,
        execution_id TEXT NOT NULL,
        ord INTEGER NOT NULL,
        doc TEXT NOT NULL
    );
    CREATE INDEX approvals_by_execution ON approvals (execution_id, ord);

    CREATE TABLE leases (
        execution_id TEXT PRIMARY KEY,
        owner TEXT NOT NULL,
        expires_at INTEGER NOT NULL
    );
    ",
];

/// The newest schema version this build knows.
pub const LATEST: u32 = MIGRATIONS.len() as u32;

/// Bring the database up to date. Refuses a database made by a newer ZORIX, because it could damage it.
pub fn migrate(conn: &mut Connection) -> Result<(), String> {
    let current: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0)).map_err(|e| e.to_string())?;
    if current > LATEST {
        return Err(format!(
            "This database was made by a newer version of ZORIX (schema {current}; this one understands up to {LATEST}). Update ZORIX to open it."
        ));
    }
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute_batch(sql).map_err(|e| format!("migration {} failed: {e}", index + 1))?;
        tx.pragma_update(None, "user_version", index as u32 + 1).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(conn: &Connection) -> u32 {
        conn.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap()
    }

    #[test]
    fn a_new_database_is_brought_to_the_latest_version_and_migrating_again_changes_nothing() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        assert_eq!(version(&conn), LATEST);
        migrate(&mut conn).unwrap();
        assert_eq!(version(&conn), LATEST);
        let tables: u32 = conn
            .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('executions','events','checkpoints','tool_calls','approvals','leases')", [], |r| r.get(0))
            .unwrap();
        assert_eq!(tables, 6);
    }

    #[test]
    fn a_database_from_a_newer_zorix_is_refused() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", LATEST + 1).unwrap();
        let error = migrate(&mut conn).unwrap_err();
        assert!(error.contains("newer version of ZORIX"), "{error}");
    }
}
