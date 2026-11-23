//! The SQLite implementation of [`AgentStore`] (ADR-0024 to ADR-0028). It opens its own connection like the other stores and
//! every query carries the tenant. A version of an agent is never changed after it is written.

use std::path::Path;
use std::sync::Arc;

use crate::errors::storage;
use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::{Connection as Db, OptionalExtension, params};
use zorix_core::agent::{MAX_AGENTS, MAX_TEAMS, MAX_VERSIONS};
use zorix_core::{
    Agent, AgentConversation, AgentId, AgentSlug, AgentSpec, AgentStore, AgentVersion, ConversationId, Error, EvalCase,
    EvalCaseId, EvalRun, EvalRunId, Result, SkillRecord, StoredCase, Team, TenantId, Timestamp,
};

/// Stores agents in the same SQLite file as everything else.
#[derive(Clone)]
pub struct SqliteAgentStore {
    conn: Arc<Mutex<Db>>,
}

fn invalid(what: impl Into<String>) -> Error {
    Error::InvalidInput(what.into())
}

fn parse<T: serde::de::DeserializeOwned>(text: &str) -> Result<T> {
    serde_json::from_str(text).map_err(|e| Error::Storage(format!("a stored record could not be read: {e}")))
}

fn encode<T: serde::Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(storage)
}

impl SqliteAgentStore {
    /// Open the database at `path` (creating and migrating it when needed) with its own connection.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Storage(format!("could not create {}: {e}", parent.display())))?;
        }
        Self::prepare(Db::open(path).map_err(storage)?, true)
    }

    /// A database in memory, for tests.
    pub fn open_in_memory() -> Result<Self> {
        Self::prepare(Db::open_in_memory().map_err(storage)?, false)
    }

    fn prepare(mut conn: Db, file: bool) -> Result<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(storage)?;
        if file {
            conn.pragma_update(None, "journal_mode", "WAL").map_err(storage)?;
            conn.pragma_update(None, "synchronous", "NORMAL").map_err(storage)?;
        }
        conn.pragma_update(None, "secure_delete", "ON").map_err(storage)?;
        crate::schema::migrate(&mut conn).map_err(Error::Storage)?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    fn with<T>(&self, work: impl FnOnce(&mut Db) -> Result<T>) -> Result<T> {
        let mut conn = self.conn.lock();
        work(&mut conn)
    }
}

type RawAgent = (String, String, String, u32, bool, i64, i64);

fn agent_from(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawAgent> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?))
}

fn build_agent(raw: RawAgent) -> Result<Agent> {
    let (id, tenant, slug, current, archived, created, updated) = raw;
    Ok(Agent {
        id: id.parse().map_err(storage)?,
        tenant: TenantId::new(tenant).map_err(storage)?,
        slug: AgentSlug::new(slug).map_err(storage)?,
        current_version: current,
        archived,
        created_at: Timestamp(created),
        updated_at: Timestamp(updated),
    })
}

const AGENT_COLUMNS: &str = "id, tenant, slug, current_version, archived, created_at, updated_at";

fn load_agent(db: &Db, tenant: &TenantId, id: AgentId) -> Result<Option<Agent>> {
    db.query_row(
        &format!("SELECT {AGENT_COLUMNS} FROM agents WHERE id = ?1 AND tenant = ?2"),
        params![id.to_string(), tenant.as_str()],
        agent_from,
    )
    .optional()
    .map_err(storage)?
    .map(build_agent)
    .transpose()
}

fn require_agent(db: &Db, tenant: &TenantId, id: AgentId) -> Result<Agent> {
    load_agent(db, tenant, id)?.ok_or_else(|| Error::Missing(format!("agent {id}")))
}

fn version_from(row: &rusqlite::Row<'_>) -> rusqlite::Result<(String, u32, String, Option<String>, i64, String)> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?))
}

fn build_version(raw: (String, u32, String, Option<String>, i64, String)) -> Result<AgentVersion> {
    let (agent, version, hash, note, created, doc) = raw;
    Ok(AgentVersion {
        agent: agent.parse().map_err(storage)?,
        version,
        hash,
        spec: parse::<AgentSpec>(&doc)?,
        note,
        created_at: Timestamp(created),
    })
}

const VERSION_COLUMNS: &str = "agent_id, version, hash, note, created_at, doc";

fn load_version(db: &Db, id: AgentId, version: u32) -> Result<Option<AgentVersion>> {
    db.query_row(
        &format!("SELECT {VERSION_COLUMNS} FROM agent_versions WHERE agent_id = ?1 AND version = ?2"),
        params![id.to_string(), version],
        version_from,
    )
    .optional()
    .map_err(storage)?
    .map(build_version)
    .transpose()
}

fn write_version(
    db: &Db,
    id: AgentId,
    version: u32,
    spec: &AgentSpec,
    note: Option<String>,
    now: Timestamp,
) -> Result<AgentVersion> {
    let hash = spec.hash();
    db.execute(
        "INSERT INTO agent_versions (agent_id, version, hash, note, created_at, doc) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id.to_string(), version, hash, note, now.millis(), encode(spec)?],
    )
    .map_err(storage)?;
    Ok(AgentVersion { agent: id, version, hash, spec: spec.clone(), note, created_at: now })
}

fn check_case_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        return Err(invalid("A case needs a name."));
    }
    Ok(())
}

#[async_trait]
impl AgentStore for SqliteAgentStore {
    async fn agents(&self, tenant: &TenantId, include_archived: bool) -> Result<Vec<Agent>> {
        self.with(|db| {
            let sql = format!(
                "SELECT {AGENT_COLUMNS} FROM agents WHERE tenant = ?1 {} ORDER BY slug",
                if include_archived { "" } else { "AND archived = 0" }
            );
            let mut statement = db.prepare(&sql).map_err(storage)?;
            let rows = statement.query_map(params![tenant.as_str()], agent_from).map_err(storage)?;
            rows.map(|r| build_agent(r.map_err(storage)?)).collect()
        })
    }

    async fn agent(&self, tenant: &TenantId, id: AgentId) -> Result<Option<Agent>> {
        self.with(|db| load_agent(db, tenant, id))
    }

    async fn agent_by_slug(&self, tenant: &TenantId, slug: &AgentSlug) -> Result<Option<Agent>> {
        self.with(|db| {
            db.query_row(
                &format!("SELECT {AGENT_COLUMNS} FROM agents WHERE tenant = ?1 AND slug = ?2"),
                params![tenant.as_str(), slug.as_str()],
                agent_from,
            )
            .optional()
            .map_err(storage)?
            .map(build_agent)
            .transpose()
        })
    }

    async fn create_agent(
        &self,
        tenant: &TenantId,
        spec: AgentSpec,
        note: Option<String>,
        now: Timestamp,
    ) -> Result<(Agent, AgentVersion)> {
        spec.validate()?;
        self.with(|db| {
            let tx = db.transaction().map_err(storage)?;
            let count: i64 = tx
                .query_row("SELECT COUNT(*) FROM agents WHERE tenant = ?1", params![tenant.as_str()], |r| r.get(0))
                .map_err(storage)?;
            if count >= MAX_AGENTS as i64 {
                return Err(invalid(format!("A tenant keeps {MAX_AGENTS} agents at most.")));
            }
            let taken: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM agents WHERE tenant = ?1 AND slug = ?2)",
                    params![tenant.as_str(), spec.slug.as_str()],
                    |r| r.get(0),
                )
                .map_err(storage)?;
            if taken {
                return Err(invalid(format!("There is already an agent called {}.", spec.slug)));
            }
            let id = AgentId::new();
            tx.execute(
                "INSERT INTO agents (id, tenant, slug, current_version, archived, created_at, updated_at) VALUES (?1, ?2, ?3, 1, 0, ?4, ?4)",
                params![id.to_string(), tenant.as_str(), spec.slug.as_str(), now.millis()],
            )
            .map_err(storage)?;
            let version = write_version(&tx, id, 1, &spec, note, now)?;
            tx.commit().map_err(storage)?;
            let agent = Agent {
                id,
                tenant: tenant.clone(),
                slug: spec.slug,
                current_version: 1,
                archived: false,
                created_at: now,
                updated_at: now,
            };
            Ok((agent, version))
        })
    }

    async fn save_version(
        &self,
        tenant: &TenantId,
        id: AgentId,
        spec: AgentSpec,
        note: Option<String>,
        now: Timestamp,
    ) -> Result<(AgentVersion, bool)> {
        spec.validate()?;
        self.with(|db| {
            let tx = db.transaction().map_err(storage)?;
            let agent = require_agent(&tx, tenant, id)?;
            if spec.slug != agent.slug {
                return Err(invalid("The name of an agent does not change."));
            }
            let current = load_version(&tx, id, agent.current_version)?
                .ok_or_else(|| Error::Storage("an agent lost its newest version".into()))?;
            if current.hash == spec.hash() {
                return Ok((current, false));
            }
            if agent.current_version as usize >= MAX_VERSIONS {
                return Err(invalid(format!("An agent keeps {MAX_VERSIONS} versions at most.")));
            }
            let next = agent.current_version + 1;
            let version = write_version(&tx, id, next, &spec, note, now)?;
            tx.execute(
                "UPDATE agents SET current_version = ?2, updated_at = ?3 WHERE id = ?1",
                params![id.to_string(), next, now.millis()],
            )
            .map_err(storage)?;
            tx.commit().map_err(storage)?;
            Ok((version, true))
        })
    }

    async fn versions(&self, tenant: &TenantId, id: AgentId) -> Result<Vec<AgentVersion>> {
        self.with(|db| {
            require_agent(db, tenant, id)?;
            let mut statement = db
                .prepare(&format!(
                    "SELECT {VERSION_COLUMNS} FROM agent_versions WHERE agent_id = ?1 ORDER BY version DESC"
                ))
                .map_err(storage)?;
            let rows = statement.query_map(params![id.to_string()], version_from).map_err(storage)?;
            rows.map(|r| build_version(r.map_err(storage)?)).collect()
        })
    }

    async fn version(&self, tenant: &TenantId, id: AgentId, version: u32) -> Result<Option<AgentVersion>> {
        self.with(|db| {
            if load_agent(db, tenant, id)?.is_none() {
                return Ok(None);
            }
            load_version(db, id, version)
        })
    }

    async fn rollback(&self, tenant: &TenantId, id: AgentId, to: u32, now: Timestamp) -> Result<AgentVersion> {
        let old =
            self.version(tenant, id, to).await?.ok_or_else(|| Error::Missing(format!("version {to} of agent {id}")))?;
        let (version, _) = self.save_version(tenant, id, old.spec, Some(format!("rollback to v{to}")), now).await?;
        Ok(version)
    }

    async fn set_archived(&self, tenant: &TenantId, id: AgentId, archived: bool) -> Result<()> {
        self.with(|db| {
            require_agent(db, tenant, id)?;
            db.execute("UPDATE agents SET archived = ?2 WHERE id = ?1", params![id.to_string(), archived])
                .map_err(storage)?;
            Ok(())
        })
    }

    async fn remove_agent(&self, tenant: &TenantId, id: AgentId) -> Result<bool> {
        self.with(|db| {
            if load_agent(db, tenant, id)?.is_none() {
                return Ok(false);
            }
            let tx = db.transaction().map_err(storage)?;
            let key = id.to_string();
            for table in ["agent_versions", "eval_cases", "eval_runs"] {
                tx.execute(&format!("DELETE FROM {table} WHERE agent_id = ?1"), params![key]).map_err(storage)?;
            }
            tx.execute("DELETE FROM agents WHERE id = ?1", params![key]).map_err(storage)?;
            tx.commit().map_err(storage)?;
            Ok(true)
        })
    }

    async fn conversations(&self, tenant: &TenantId, id: AgentId, limit: usize) -> Result<Vec<AgentConversation>> {
        self.with(|db| {
            let mut statement = db
                .prepare(
                    "SELECT e.conversation, COUNT(*), MAX(e.created_at),
                        (SELECT json_extract(e2.doc, '$.request.prompt') FROM executions e2
                         WHERE e2.tenant = e.tenant AND e2.agent = e.agent AND e2.conversation = e.conversation
                         ORDER BY e2.created_at ASC, e2.id ASC LIMIT 1)
                     FROM executions e
                     WHERE e.tenant = ?1 AND e.agent = ?2 AND e.conversation IS NOT NULL
                     GROUP BY e.conversation ORDER BY MAX(e.created_at) DESC LIMIT ?3",
                )
                .map_err(storage)?;
            let rows = statement
                .query_map(params![tenant.as_str(), id.to_string(), limit.max(1) as i64], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, u32>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                })
                .map_err(storage)?;
            rows.map(|r| {
                let (conversation, runs, last, title) = r.map_err(storage)?;
                Ok(AgentConversation {
                    conversation: ConversationId::new(conversation).map_err(storage)?,
                    title: title.unwrap_or_default().chars().take(80).collect(),
                    runs,
                    last_at: Timestamp(last),
                })
            })
            .collect()
        })
    }

    async fn skills(&self, tenant: &TenantId) -> Result<Vec<SkillRecord>> {
        self.with(|db| {
            let mut statement = db
                .prepare(
                    "SELECT v.doc FROM skills s JOIN skill_versions v
                       ON v.tenant = s.tenant AND v.name = s.name AND v.hash = s.current_hash
                     WHERE s.tenant = ?1 ORDER BY s.name",
                )
                .map_err(storage)?;
            let rows = statement.query_map(params![tenant.as_str()], |r| r.get::<_, String>(0)).map_err(storage)?;
            rows.map(|r| parse(&r.map_err(storage)?)).collect()
        })
    }

    async fn skill(&self, tenant: &TenantId, name: &str) -> Result<Option<SkillRecord>> {
        self.with(|db| {
            db.query_row(
                "SELECT v.doc FROM skills s JOIN skill_versions v
                   ON v.tenant = s.tenant AND v.name = s.name AND v.hash = s.current_hash
                 WHERE s.tenant = ?1 AND s.name = ?2",
                params![tenant.as_str(), name],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(storage)?
            .map(|doc| parse(&doc))
            .transpose()
        })
    }

    async fn skill_version(&self, tenant: &TenantId, name: &str, hash: &str) -> Result<Option<SkillRecord>> {
        self.with(|db| {
            db.query_row(
                "SELECT doc FROM skill_versions WHERE tenant = ?1 AND name = ?2 AND hash = ?3",
                params![tenant.as_str(), name, hash],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(storage)?
            .map(|doc| parse(&doc))
            .transpose()
        })
    }

    async fn record_skill(&self, tenant: &TenantId, record: SkillRecord) -> Result<()> {
        if !zorix_core::valid_skill_name(&record.name) {
            return Err(invalid(format!("“{}” is not a skill name.", record.name)));
        }
        self.with(|db| {
            let tx = db.transaction().map_err(storage)?;
            tx.execute(
                "INSERT OR REPLACE INTO skill_versions (tenant, name, hash, imported_at, doc) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![tenant.as_str(), record.name, record.hash, record.imported_at.millis(), encode(&record)?],
            )
            .map_err(storage)?;
            tx.execute(
                "INSERT INTO skills (tenant, name, current_hash, updated_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (tenant, name) DO UPDATE SET current_hash = excluded.current_hash, updated_at = excluded.updated_at",
                params![tenant.as_str(), record.name, record.hash, record.imported_at.millis()],
            )
            .map_err(storage)?;
            tx.commit().map_err(storage)?;
            Ok(())
        })
    }

    async fn remove_skill(&self, tenant: &TenantId, name: &str) -> Result<bool> {
        self.with(|db| {
            let tx = db.transaction().map_err(storage)?;
            tx.execute("DELETE FROM skill_versions WHERE tenant = ?1 AND name = ?2", params![tenant.as_str(), name])
                .map_err(storage)?;
            let removed = tx
                .execute("DELETE FROM skills WHERE tenant = ?1 AND name = ?2", params![tenant.as_str(), name])
                .map_err(storage)?;
            tx.commit().map_err(storage)?;
            Ok(removed > 0)
        })
    }

    async fn referenced_skill_hashes(&self, tenant: &TenantId) -> Result<Vec<String>> {
        self.with(|db| {
            let mut hashes: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
            let mut statement = db.prepare("SELECT current_hash FROM skills WHERE tenant = ?1").map_err(storage)?;
            for row in statement.query_map(params![tenant.as_str()], |r| r.get::<_, String>(0)).map_err(storage)? {
                hashes.insert(row.map_err(storage)?);
            }
            let mut statement = db
                .prepare("SELECT v.doc FROM agent_versions v JOIN agents a ON a.id = v.agent_id WHERE a.tenant = ?1")
                .map_err(storage)?;
            for row in statement.query_map(params![tenant.as_str()], |r| r.get::<_, String>(0)).map_err(storage)? {
                let spec: AgentSpec = parse(&row.map_err(storage)?)?;
                hashes.extend(spec.skills.into_iter().map(|pin| pin.hash));
            }
            Ok(hashes.into_iter().collect())
        })
    }

    async fn eval_cases(&self, tenant: &TenantId, agent: AgentId) -> Result<Vec<StoredCase>> {
        self.with(|db| {
            require_agent(db, tenant, agent)?;
            let mut statement = db
                .prepare("SELECT id, updated_at, doc FROM eval_cases WHERE tenant = ?1 AND agent_id = ?2 ORDER BY name")
                .map_err(storage)?;
            let rows = statement
                .query_map(params![tenant.as_str(), agent.to_string()], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?))
                })
                .map_err(storage)?;
            rows.map(|r| {
                let (id, updated, doc) = r.map_err(storage)?;
                Ok(StoredCase {
                    id: id.parse().map_err(storage)?,
                    agent,
                    case: parse::<EvalCase>(&doc)?,
                    updated_at: Timestamp(updated),
                })
            })
            .collect()
        })
    }

    async fn put_eval_case(
        &self,
        tenant: &TenantId,
        agent: AgentId,
        case: EvalCase,
        now: Timestamp,
    ) -> Result<StoredCase> {
        check_case_name(&case.name)?;
        case.validate()?;
        self.with(|db| {
            require_agent(db, tenant, agent)?;
            let existing: Option<String> = db
                .query_row(
                    "SELECT id FROM eval_cases WHERE agent_id = ?1 AND name = ?2",
                    params![agent.to_string(), case.name],
                    |r| r.get(0),
                )
                .optional()
                .map_err(storage)?;
            let id = match existing {
                Some(id) => id.parse::<EvalCaseId>().map_err(storage)?,
                None => EvalCaseId::new(),
            };
            db.execute(
                "INSERT INTO eval_cases (id, tenant, agent_id, name, updated_at, doc) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (id) DO UPDATE SET updated_at = excluded.updated_at, doc = excluded.doc",
                params![id.to_string(), tenant.as_str(), agent.to_string(), case.name, now.millis(), encode(&case)?],
            )
            .map_err(storage)?;
            Ok(StoredCase { id, agent, case, updated_at: now })
        })
    }

    async fn remove_eval_case(&self, tenant: &TenantId, agent: AgentId, name: &str) -> Result<bool> {
        self.with(|db| {
            require_agent(db, tenant, agent)?;
            let removed = db
                .execute("DELETE FROM eval_cases WHERE agent_id = ?1 AND name = ?2", params![agent.to_string(), name])
                .map_err(storage)?;
            Ok(removed > 0)
        })
    }

    async fn save_eval_run(&self, tenant: &TenantId, run: &EvalRun) -> Result<()> {
        self.with(|db| {
            require_agent(db, tenant, run.agent)?;
            db.execute(
                "INSERT INTO eval_runs (id, tenant, agent_id, version, model, status, started_at, doc)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT (id) DO UPDATE SET status = excluded.status, doc = excluded.doc",
                params![
                    run.id.to_string(),
                    tenant.as_str(),
                    run.agent.to_string(),
                    run.version,
                    run.model,
                    run.status,
                    run.started_at.millis(),
                    encode(run)?
                ],
            )
            .map_err(storage)?;
            Ok(())
        })
    }

    async fn eval_run(&self, tenant: &TenantId, id: EvalRunId) -> Result<Option<EvalRun>> {
        self.with(|db| {
            db.query_row(
                "SELECT doc FROM eval_runs WHERE id = ?1 AND tenant = ?2",
                params![id.to_string(), tenant.as_str()],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(storage)?
            .map(|doc| parse(&doc))
            .transpose()
        })
    }

    async fn eval_runs(&self, tenant: &TenantId, agent: AgentId, limit: usize) -> Result<Vec<EvalRun>> {
        self.with(|db| {
            require_agent(db, tenant, agent)?;
            let mut statement = db
                .prepare(
                    "SELECT doc FROM eval_runs WHERE tenant = ?1 AND agent_id = ?2 ORDER BY started_at DESC, id DESC LIMIT ?3",
                )
                .map_err(storage)?;
            let rows = statement
                .query_map(params![tenant.as_str(), agent.to_string(), limit.max(1) as i64], |r| r.get::<_, String>(0))
                .map_err(storage)?;
            rows.map(|r| parse(&r.map_err(storage)?)).collect()
        })
    }

    async fn teams(&self, tenant: &TenantId) -> Result<Vec<Team>> {
        self.with(|db| {
            let mut statement = db.prepare("SELECT doc FROM teams WHERE tenant = ?1 ORDER BY slug").map_err(storage)?;
            let rows = statement.query_map(params![tenant.as_str()], |r| r.get::<_, String>(0)).map_err(storage)?;
            rows.map(|r| parse(&r.map_err(storage)?)).collect()
        })
    }

    async fn team(&self, tenant: &TenantId, slug: &AgentSlug) -> Result<Option<Team>> {
        self.with(|db| {
            let doc: Option<String> = db
                .query_row(
                    "SELECT doc FROM teams WHERE tenant = ?1 AND slug = ?2",
                    params![tenant.as_str(), slug.as_str()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(storage)?;
            doc.map(|d| parse(&d)).transpose()
        })
    }

    async fn put_team(&self, tenant: &TenantId, mut team: Team) -> Result<Team> {
        team.tenant = tenant.clone();
        team.validate()?;
        self.with(|db| {
            let existing: i64 = db
                .query_row(
                    "SELECT COUNT(*) FROM teams WHERE tenant = ?1 AND slug != ?2",
                    params![tenant.as_str(), team.slug.as_str()],
                    |r| r.get(0),
                )
                .map_err(storage)?;
            let known: Option<i64> = db
                .query_row(
                    "SELECT 1 FROM teams WHERE tenant = ?1 AND slug = ?2",
                    params![tenant.as_str(), team.slug.as_str()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(storage)?;
            if known.is_none() && existing as usize >= MAX_TEAMS {
                return Err(invalid(format!("There are {MAX_TEAMS} teams already, which is the most.")));
            }
            db.execute(
                "INSERT INTO teams (tenant, slug, updated_at, doc) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (tenant, slug) DO UPDATE SET updated_at = excluded.updated_at, doc = excluded.doc",
                params![tenant.as_str(), team.slug.as_str(), team.updated_at.millis(), encode(&team)?],
            )
            .map_err(storage)?;
            Ok(team.clone())
        })
    }

    async fn remove_team(&self, tenant: &TenantId, slug: &AgentSlug) -> Result<bool> {
        self.with(|db| {
            let n = db
                .execute("DELETE FROM teams WHERE tenant = ?1 AND slug = ?2", params![tenant.as_str(), slug.as_str()])
                .map_err(storage)?;
            Ok(n > 0)
        })
    }
}
