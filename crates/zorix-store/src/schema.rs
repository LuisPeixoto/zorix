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
    // 2: memory (Phase 3). One full-text table per corpus, so BM25 statistics never mix. Vectors are keyed by the hash of
    // the text, so the same text is never embedded twice. Written once, before the 0.3.0 release.
    "
    CREATE TABLE memory_items (
        id TEXT PRIMARY KEY,
        tenant TEXT NOT NULL,
        scope TEXT NOT NULL,
        scope_key TEXT NOT NULL DEFAULT '',
        kind TEXT NOT NULL,
        subject TEXT,
        content TEXT NOT NULL,
        content_hash TEXT NOT NULL,
        status TEXT NOT NULL,
        superseded_by TEXT,
        origin TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        confirmed_at INTEGER NOT NULL,
        valid_until INTEGER,
        last_used_at INTEGER,
        use_count INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX memory_by_scope ON memory_items (tenant, scope, scope_key, status);
    CREATE UNIQUE INDEX memory_dedupe ON memory_items (tenant, scope, scope_key, content_hash) WHERE status = 'active';
    CREATE INDEX memory_by_subject ON memory_items (tenant, scope, scope_key, subject) WHERE subject IS NOT NULL AND status = 'active';
    CREATE VIRTUAL TABLE memory_fts USING fts5(
        content, subject, content = 'memory_items', content_rowid = 'rowid',
        tokenize = \"unicode61 remove_diacritics 2 tokenchars '_'\"
    );
    CREATE TRIGGER memory_ai AFTER INSERT ON memory_items BEGIN
        INSERT INTO memory_fts (rowid, content, subject) VALUES (new.rowid, new.content, new.subject);
    END;
    CREATE TRIGGER memory_ad AFTER DELETE ON memory_items BEGIN
        INSERT INTO memory_fts (memory_fts, rowid, content, subject) VALUES ('delete', old.rowid, old.content, old.subject);
    END;
    CREATE TRIGGER memory_au AFTER UPDATE ON memory_items BEGIN
        INSERT INTO memory_fts (memory_fts, rowid, content, subject) VALUES ('delete', old.rowid, old.content, old.subject);
        INSERT INTO memory_fts (rowid, content, subject) VALUES (new.rowid, new.content, new.subject);
    END;

    CREATE TABLE knowledge_roots (
        id TEXT PRIMARY KEY,
        tenant TEXT NOT NULL,
        scope TEXT NOT NULL,
        scope_key TEXT NOT NULL DEFAULT '',
        path TEXT NOT NULL,
        include TEXT NOT NULL DEFAULT '[]',
        exclude TEXT NOT NULL DEFAULT '[]',
        max_file_bytes INTEGER NOT NULL,
        enabled INTEGER NOT NULL DEFAULT 1,
        auto_sync INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL,
        last_sync_at INTEGER,
        last_sync TEXT
    );
    CREATE UNIQUE INDEX roots_by_path ON knowledge_roots (tenant, path);
    CREATE TABLE knowledge_documents (
        id TEXT PRIMARY KEY,
        root_id TEXT NOT NULL,
        rel_path TEXT NOT NULL,
        kind TEXT NOT NULL,
        size INTEGER NOT NULL,
        mtime_ns INTEGER NOT NULL,
        content_hash TEXT NOT NULL,
        status TEXT NOT NULL,
        error TEXT,
        chunks INTEGER NOT NULL DEFAULT 0,
        indexed_at INTEGER NOT NULL
    );
    CREATE UNIQUE INDEX documents_by_path ON knowledge_documents (root_id, rel_path);
    CREATE TABLE knowledge_chunks (
        id INTEGER PRIMARY KEY,
        document_id TEXT NOT NULL,
        ord INTEGER NOT NULL,
        text TEXT NOT NULL,
        text_hash TEXT NOT NULL,
        heading TEXT,
        line_start INTEGER,
        line_end INTEGER,
        page INTEGER,
        tokens INTEGER NOT NULL
    );
    CREATE INDEX chunks_by_document ON knowledge_chunks (document_id, ord);
    CREATE INDEX chunks_by_hash ON knowledge_chunks (text_hash);
    CREATE VIRTUAL TABLE chunks_fts USING fts5(
        text, heading, content = 'knowledge_chunks', content_rowid = 'id',
        tokenize = \"unicode61 remove_diacritics 2 tokenchars '_'\"
    );
    CREATE TRIGGER chunks_ai AFTER INSERT ON knowledge_chunks BEGIN
        INSERT INTO chunks_fts (rowid, text, heading) VALUES (new.id, new.text, new.heading);
    END;
    CREATE TRIGGER chunks_ad AFTER DELETE ON knowledge_chunks BEGIN
        INSERT INTO chunks_fts (chunks_fts, rowid, text, heading) VALUES ('delete', old.id, old.text, old.heading);
    END;

    CREATE TABLE operations (
        execution_id TEXT PRIMARY KEY,
        tenant TEXT NOT NULL,
        project TEXT,
        conversation TEXT,
        state TEXT NOT NULL,
        finished_at INTEGER NOT NULL,
        text TEXT NOT NULL,
        text_hash TEXT NOT NULL,
        doc TEXT NOT NULL
    );
    CREATE INDEX operations_by_tenant ON operations (tenant, finished_at DESC);
    CREATE VIRTUAL TABLE operations_fts USING fts5(
        text, content = 'operations', content_rowid = 'rowid',
        tokenize = \"unicode61 remove_diacritics 2 tokenchars '_'\"
    );
    CREATE TRIGGER operations_ai AFTER INSERT ON operations BEGIN
        INSERT INTO operations_fts (rowid, text) VALUES (new.rowid, new.text);
    END;
    CREATE TRIGGER operations_ad AFTER DELETE ON operations BEGIN
        INSERT INTO operations_fts (operations_fts, rowid, text) VALUES ('delete', old.rowid, old.text);
    END;

    CREATE TABLE vectors (
        model TEXT NOT NULL,
        text_hash TEXT NOT NULL,
        dims INTEGER NOT NULL,
        vector BLOB NOT NULL,
        PRIMARY KEY (model, text_hash)
    ) WITHOUT ROWID;

    CREATE TABLE context_snapshots (
        execution_id TEXT PRIMARY KEY,
        tenant TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        doc TEXT NOT NULL
    );
    CREATE TABLE snapshot_items (
        execution_id TEXT NOT NULL,
        item TEXT NOT NULL,
        PRIMARY KEY (item, execution_id)
    ) WITHOUT ROWID;

    CREATE TABLE token_calibration (
        model TEXT PRIMARY KEY,
        factor REAL NOT NULL,
        samples INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    );
    CREATE TABLE memory_settings (
        tenant TEXT PRIMARY KEY,
        doc TEXT NOT NULL
    );
    ",
    // 3: integrations. MCP servers a person added, what was approved about each tool, and connections to services. No secret
    // is ever stored here: a server or connection only says that one is kept in the keychain.
    "
    CREATE TABLE mcp_servers (
        id TEXT PRIMARY KEY,
        tenant TEXT NOT NULL,
        name TEXT NOT NULL,
        enabled INTEGER NOT NULL,
        era TEXT,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        doc TEXT NOT NULL
    );
    CREATE UNIQUE INDEX mcp_servers_by_name ON mcp_servers (tenant, name);

    CREATE TABLE mcp_tool_pins (
        server_id TEXT NOT NULL,
        tool TEXT NOT NULL,
        listed INTEGER NOT NULL,
        status TEXT NOT NULL,
        trust TEXT NOT NULL,
        approved_hash TEXT,
        approved TEXT,
        seen_hash TEXT NOT NULL,
        seen TEXT NOT NULL,
        approved_by TEXT,
        approved_at INTEGER,
        updated_at INTEGER NOT NULL,
        PRIMARY KEY (server_id, tool)
    ) WITHOUT ROWID;

    CREATE TABLE connections (
        id TEXT PRIMARY KEY,
        tenant TEXT NOT NULL,
        kind TEXT NOT NULL,
        name TEXT NOT NULL,
        settings TEXT NOT NULL,
        has_secret INTEGER NOT NULL,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        test_at INTEGER,
        test_ok INTEGER,
        test_note TEXT
    );
    CREATE UNIQUE INDEX connections_by_name ON connections (tenant, name);
    ",
    // 4: agents. What an agent is, every version of it (immutable), the skills it can load, and how it was evaluated. The
    // executions table learns which agent a run came from; runs stored before this have none.
    "
    ALTER TABLE executions ADD COLUMN agent TEXT;
    CREATE INDEX executions_by_agent ON executions (tenant, agent, created_at DESC) WHERE agent IS NOT NULL;

    CREATE TABLE agents (
        id TEXT PRIMARY KEY,
        tenant TEXT NOT NULL,
        slug TEXT NOT NULL,
        current_version INTEGER NOT NULL,
        archived INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    );
    CREATE UNIQUE INDEX agents_by_slug ON agents (tenant, slug);

    CREATE TABLE agent_versions (
        agent_id TEXT NOT NULL,
        version INTEGER NOT NULL,
        hash TEXT NOT NULL,
        note TEXT,
        created_at INTEGER NOT NULL,
        doc TEXT NOT NULL,
        PRIMARY KEY (agent_id, version)
    ) WITHOUT ROWID;

    CREATE TABLE skills (
        tenant TEXT NOT NULL,
        name TEXT NOT NULL,
        current_hash TEXT NOT NULL,
        updated_at INTEGER NOT NULL,
        PRIMARY KEY (tenant, name)
    ) WITHOUT ROWID;
    CREATE TABLE skill_versions (
        tenant TEXT NOT NULL,
        name TEXT NOT NULL,
        hash TEXT NOT NULL,
        imported_at INTEGER NOT NULL,
        doc TEXT NOT NULL,
        PRIMARY KEY (tenant, name, hash)
    ) WITHOUT ROWID;

    CREATE TABLE eval_cases (
        id TEXT PRIMARY KEY,
        tenant TEXT NOT NULL,
        agent_id TEXT NOT NULL,
        name TEXT NOT NULL,
        updated_at INTEGER NOT NULL,
        doc TEXT NOT NULL
    );
    CREATE UNIQUE INDEX eval_cases_by_name ON eval_cases (agent_id, name);
    CREATE TABLE eval_runs (
        id TEXT PRIMARY KEY,
        tenant TEXT NOT NULL,
        agent_id TEXT NOT NULL,
        version INTEGER NOT NULL,
        model TEXT NOT NULL,
        status TEXT NOT NULL,
        started_at INTEGER NOT NULL,
        doc TEXT NOT NULL
    );
    CREATE INDEX eval_runs_by_agent ON eval_runs (tenant, agent_id, started_at DESC);
    ",
    // 5: orchestration. Which execution started which, so a tree is a query and not a scan, and teams. Runs stored before this
    // have no parent unless their document says so; the columns are filled from the document.
    "
    ALTER TABLE executions ADD COLUMN parent TEXT;
    ALTER TABLE executions ADD COLUMN root TEXT;
    UPDATE executions SET parent = json_extract(doc, '$.parent'), root = json_extract(doc, '$.root');
    CREATE INDEX executions_by_parent ON executions (parent, created_at) WHERE parent IS NOT NULL;
    CREATE INDEX executions_by_root ON executions (root) WHERE root IS NOT NULL;

    CREATE TABLE teams (
        tenant TEXT NOT NULL,
        slug TEXT NOT NULL,
        updated_at INTEGER NOT NULL,
        doc TEXT NOT NULL,
        PRIMARY KEY (tenant, slug)
    ) WITHOUT ROWID;
    ",
    // 6: workflows. A workflow row holds the draft; published versions never change. A run keeps its whole record in `doc` with a
    // revision for optimistic writes; waits are the people and timers a run is stopped for.
    "
    CREATE TABLE workflows (
        id TEXT PRIMARY KEY,
        tenant TEXT NOT NULL,
        slug TEXT NOT NULL,
        enabled INTEGER NOT NULL DEFAULT 0,
        live_version INTEGER,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        doc TEXT NOT NULL
    );
    CREATE UNIQUE INDEX workflows_by_slug ON workflows (tenant, slug);

    CREATE TABLE workflow_versions (
        workflow_id TEXT NOT NULL,
        version INTEGER NOT NULL,
        hash TEXT NOT NULL,
        note TEXT,
        created_at INTEGER NOT NULL,
        doc TEXT NOT NULL,
        PRIMARY KEY (workflow_id, version)
    ) WITHOUT ROWID;

    CREATE TABLE workflow_runs (
        id TEXT PRIMARY KEY,
        tenant TEXT NOT NULL,
        workflow_id TEXT NOT NULL,
        state TEXT NOT NULL,
        started_at INTEGER NOT NULL,
        revision INTEGER NOT NULL,
        doc TEXT NOT NULL
    );
    CREATE INDEX workflow_runs_by_workflow ON workflow_runs (tenant, workflow_id, started_at DESC);
    CREATE INDEX workflow_runs_by_state ON workflow_runs (tenant, state);

    CREATE TABLE workflow_waits (
        id TEXT PRIMARY KEY,
        tenant TEXT NOT NULL,
        run_id TEXT NOT NULL,
        status TEXT NOT NULL,
        due_at INTEGER,
        created_at INTEGER NOT NULL,
        doc TEXT NOT NULL
    );
    CREATE INDEX workflow_waits_by_status ON workflow_waits (tenant, status, created_at);
    CREATE INDEX workflow_waits_by_run ON workflow_waits (run_id);

    CREATE TABLE workflow_triggers (
        tenant TEXT NOT NULL,
        workflow_id TEXT NOT NULL,
        step TEXT NOT NULL,
        doc TEXT NOT NULL,
        PRIMARY KEY (tenant, workflow_id, step)
    ) WITHOUT ROWID;

    CREATE TABLE workflow_meta (
        tenant TEXT NOT NULL,
        key TEXT NOT NULL,
        value TEXT NOT NULL,
        PRIMARY KEY (tenant, key)
    ) WITHOUT ROWID;
    ",
    // 7: projects and tasks. A project and a task keep their whole record in `doc`; a task has a revision for optimistic writes and
    // the project holds the counter that numbers its tasks, so a key is never reused. `project_events` is the history.
    "
    CREATE TABLE projects (
        id TEXT PRIMARY KEY,
        tenant TEXT NOT NULL,
        key TEXT NOT NULL,
        archived INTEGER NOT NULL DEFAULT 0,
        next_task INTEGER NOT NULL DEFAULT 1,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        doc TEXT NOT NULL
    );
    CREATE UNIQUE INDEX projects_by_key ON projects (tenant, key);

    CREATE TABLE tasks (
        id TEXT PRIMARY KEY,
        tenant TEXT NOT NULL,
        project_id TEXT NOT NULL,
        key TEXT NOT NULL,
        state TEXT NOT NULL,
        rank INTEGER NOT NULL,
        revision INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        doc TEXT NOT NULL
    );
    CREATE UNIQUE INDEX tasks_by_key ON tasks (tenant, key);
    CREATE INDEX tasks_by_project ON tasks (tenant, project_id, rank);
    CREATE INDEX tasks_by_state ON tasks (tenant, state);

    CREATE TABLE project_events (
        tenant TEXT NOT NULL,
        project_id TEXT NOT NULL,
        seq INTEGER NOT NULL,
        task TEXT,
        at INTEGER NOT NULL,
        doc TEXT NOT NULL,
        PRIMARY KEY (project_id, seq)
    ) WITHOUT ROWID;
    CREATE INDEX project_events_by_task ON project_events (project_id, task, seq);
    ",
    // 8: the workforce. A worker, a report and an event keep their whole record in `doc`; the settings (with the departments) are
    // one record per tenant. A worker has a revision for optimistic writes, and the agent it is a record over is its key.
    "
    CREATE TABLE org_settings (
        tenant TEXT PRIMARY KEY,
        doc TEXT NOT NULL
    );

    CREATE TABLE workers (
        tenant TEXT NOT NULL,
        agent TEXT NOT NULL,
        revision INTEGER NOT NULL,
        hired_at INTEGER NOT NULL,
        doc TEXT NOT NULL,
        PRIMARY KEY (tenant, agent)
    ) WITHOUT ROWID;

    CREATE TABLE org_reports (
        id TEXT PRIMARY KEY,
        tenant TEXT NOT NULL,
        worker TEXT NOT NULL,
        status TEXT NOT NULL,
        started_at INTEGER NOT NULL,
        doc TEXT NOT NULL
    );
    CREATE INDEX org_reports_by_worker ON org_reports (tenant, worker, started_at DESC);
    CREATE INDEX org_reports_by_status ON org_reports (tenant, status);

    CREATE TABLE org_events (
        tenant TEXT NOT NULL,
        seq INTEGER NOT NULL,
        worker TEXT,
        at INTEGER NOT NULL,
        doc TEXT NOT NULL,
        PRIMARY KEY (tenant, seq)
    ) WITHOUT ROWID;
    CREATE INDEX org_events_by_worker ON org_events (tenant, worker, seq);
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
    fn a_database_of_version_0_5_upgrades_to_trees_and_teams_and_fills_the_parent_from_the_document() {
        let mut conn = Connection::open_in_memory().unwrap();
        for (index, sql) in MIGRATIONS.iter().enumerate().take(4) {
            conn.execute_batch(sql).unwrap();
            conn.pragma_update(None, "user_version", index as u32 + 1).unwrap();
        }
        for (id, doc) in [("e1", "{}"), ("e2", r#"{"parent":"e1","root":"e1"}"#)] {
            conn.execute(
                "INSERT INTO executions (id, tenant, conversation, state, version, idempotency_key, created_at, doc) VALUES (?1,'local',NULL,'completed',1,NULL,1,?2)",
                rusqlite::params![id, doc],
            )
            .unwrap();
        }
        migrate(&mut conn).unwrap();
        assert_eq!(version(&conn), LATEST);
        let rows: Vec<(String, Option<String>, Option<String>)> = conn
            .prepare("SELECT id, parent, root FROM executions ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(rows[0], ("e1".into(), None, None));
        assert_eq!(rows[1], ("e2".into(), Some("e1".into()), Some("e1".into())));
        let teams: u32 = conn
            .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'teams'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(teams, 1);
    }

    #[test]
    fn a_database_of_version_0_4_upgrades_to_agents_and_keeps_its_runs() {
        let mut conn = Connection::open_in_memory().unwrap();
        for (index, sql) in MIGRATIONS.iter().enumerate().take(3) {
            conn.execute_batch(sql).unwrap();
            conn.pragma_update(None, "user_version", index as u32 + 1).unwrap();
        }
        conn.execute(
            "INSERT INTO executions (id, tenant, conversation, state, version, idempotency_key, created_at, doc) VALUES ('e1','local',NULL,'completed',1,NULL,1,'{}')",
            [],
        )
        .unwrap();
        migrate(&mut conn).unwrap();
        assert_eq!(version(&conn), LATEST);
        let (count, agent): (u32, Option<String>) =
            conn.query_row("SELECT COUNT(*), MAX(agent) FROM executions", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((count, agent), (1, None), "the old run stays, with no agent");
        let tables: u32 = conn
            .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('agents','agent_versions','skills','skill_versions','eval_cases','eval_runs')", [], |r| r.get(0))
            .unwrap();
        assert_eq!(tables, 6);
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
        let memory: u32 = conn
            .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('memory_items','knowledge_roots','knowledge_documents','knowledge_chunks','operations','vectors','context_snapshots','snapshot_items','token_calibration','memory_settings','memory_fts','chunks_fts','operations_fts')", [], |r| r.get(0))
            .unwrap();
        assert_eq!(memory, 13, "FTS5 is compiled into the bundled SQLite on every platform");
        let integrations: u32 = conn
            .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('mcp_servers','mcp_tool_pins','connections')", [], |r| r.get(0))
            .unwrap();
        assert_eq!(integrations, 3);
    }

    #[test]
    fn a_database_from_zorix_0_2_is_upgraded_and_keeps_its_executions() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATIONS[0]).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute("INSERT INTO executions (id, tenant, state, version, created_at, doc) VALUES ('e1','local','completed',1,5,'{}')", []).unwrap();
        migrate(&mut conn).unwrap();
        assert_eq!(version(&conn), LATEST);
        let kept: u32 = conn.query_row("SELECT COUNT(*) FROM executions WHERE id = 'e1'", [], |r| r.get(0)).unwrap();
        assert_eq!(kept, 1);
        conn.execute("INSERT INTO operations (execution_id, tenant, state, finished_at, text, text_hash, doc) VALUES ('e1','local','completed',9,'hello world','h','{}')", []).unwrap();
        let found: u32 = conn
            .query_row("SELECT COUNT(*) FROM operations_fts WHERE operations_fts MATCH '\"hello\"'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(found, 1);
    }

    #[test]
    fn a_database_from_zorix_0_3_is_upgraded_and_keeps_its_memory() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATIONS[0]).unwrap();
        conn.execute_batch(MIGRATIONS[1]).unwrap();
        conn.pragma_update(None, "user_version", 2).unwrap();
        conn.execute("INSERT INTO memory_settings (tenant, doc) VALUES ('local', '{}')", []).unwrap();
        migrate(&mut conn).unwrap();
        assert_eq!(version(&conn), LATEST);
        let kept: u32 = conn.query_row("SELECT COUNT(*) FROM memory_settings", [], |r| r.get(0)).unwrap();
        assert_eq!(kept, 1);
        conn.execute("INSERT INTO connections (id, tenant, kind, name, settings, has_secret, created_at, updated_at) VALUES ('c1','local','github','gh','{}',1,1,1)", []).unwrap();
    }

    #[test]
    fn a_database_from_a_newer_zorix_is_refused() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", LATEST + 1).unwrap();
        let error = migrate(&mut conn).unwrap_err();
        assert!(error.contains("newer version of ZORIX"), "{error}");
    }

    #[test]
    fn a_database_of_schema_5_upgrades_to_workflows_and_keeps_its_data() {
        let mut conn = Connection::open_in_memory().unwrap();
        for (index, sql) in MIGRATIONS.iter().enumerate().take(5) {
            conn.execute_batch(sql).unwrap();
            conn.pragma_update(None, "user_version", index as u32 + 1).unwrap();
        }
        conn.execute(
            "INSERT INTO executions (id, tenant, conversation, state, version, idempotency_key, created_at, doc) VALUES ('e1','local',NULL,'completed',1,NULL,1,'{}')",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO teams (tenant, slug, updated_at, doc) VALUES ('local','desk',1,'{}')", []).unwrap();
        migrate(&mut conn).unwrap();
        assert_eq!(version(&conn), LATEST);
        for table in
            ["workflows", "workflow_versions", "workflow_runs", "workflow_waits", "workflow_triggers", "workflow_meta"]
        {
            let found: u32 = conn
                .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1", [table], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(found, 1, "{table} was not created");
        }
        let runs: u32 = conn.query_row("SELECT COUNT(*) FROM executions", [], |r| r.get(0)).unwrap();
        let teams: u32 = conn.query_row("SELECT COUNT(*) FROM teams", [], |r| r.get(0)).unwrap();
        assert_eq!((runs, teams), (1, 1));
    }

    #[test]
    fn a_database_of_schema_6_upgrades_to_projects_and_keeps_its_data() {
        let mut conn = Connection::open_in_memory().unwrap();
        for (index, sql) in MIGRATIONS.iter().enumerate().take(6) {
            conn.execute_batch(sql).unwrap();
            conn.pragma_update(None, "user_version", index as u32 + 1).unwrap();
        }
        conn.execute(
            "INSERT INTO workflows (id, tenant, slug, enabled, live_version, created_at, updated_at, doc) VALUES ('w1','local','daily',0,NULL,1,1,'{}')",
            [],
        )
        .unwrap();
        migrate(&mut conn).unwrap();
        assert_eq!(version(&conn), LATEST);
        for table in ["projects", "tasks", "project_events"] {
            let found: u32 = conn
                .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1", [table], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(found, 1, "{table} was not created");
        }
        let workflows: u32 = conn.query_row("SELECT COUNT(*) FROM workflows", [], |r| r.get(0)).unwrap();
        assert_eq!(workflows, 1);
    }

    #[test]
    fn a_database_of_schema_7_upgrades_to_the_workforce_and_keeps_its_data() {
        let mut conn = Connection::open_in_memory().unwrap();
        for (index, sql) in MIGRATIONS.iter().enumerate().take(7) {
            conn.execute_batch(sql).unwrap();
            conn.pragma_update(None, "user_version", index as u32 + 1).unwrap();
        }
        conn.execute(
            "INSERT INTO projects (id, tenant, key, archived, next_task, created_at, updated_at, doc) VALUES ('p1','local','OPS',0,1,1,1,'{}')",
            [],
        )
        .unwrap();
        migrate(&mut conn).unwrap();
        assert_eq!(version(&conn), LATEST);
        for table in ["org_settings", "workers", "org_reports", "org_events"] {
            let found: u32 = conn
                .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1", [table], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(found, 1, "{table} was not created");
        }
        let projects: u32 = conn.query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0)).unwrap();
        assert_eq!(projects, 1);
    }
}
