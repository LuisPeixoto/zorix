//! The tools through which a model reads and proposes memory.
//!
//! `memory.remember` only *proposes*: it always needs a person's approval, and an automatic approval mode cannot give it
//! (ADR-0013). `memory.search` reads.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use zorix_core::{
    ApprovalStatus, Clock, ExecutionId, ExecutionRepository, MemoryFilter, MemoryId, MemoryKind, MemoryOrigin,
    MemoryStore, NewMemory, Remembered, Scope, Timestamp, Tool, ToolAnnotations, ToolCallStatus, ToolContext,
    ToolError, ToolRegistry, ToolSpec, Visibility, redact_text,
};

use crate::embedder::Embedder;
use crate::retrieve::hybrid_search;
use crate::text::{cut, date};

/// Most memories one execution may propose.
pub const MAX_PROPOSALS_PER_EXECUTION: usize = 10;
/// Most results of a search.
pub const MAX_RESULTS: usize = 10;

fn failed(message: impl Into<String>) -> ToolError {
    ToolError::Failed(message.into())
}

fn visibility(ctx: &ToolContext) -> Visibility {
    Visibility {
        tenant: ctx.tenant.clone(),
        project: ctx.workspace_root.as_ref().map(|p| p.to_string_lossy().into_owned()),
        conversation: ctx.conversation.clone(),
    }
}

/// Milliseconds since 1970 of the end of a `YYYY-MM-DD` day (UTC), or `None` when it is not a real date.
pub fn end_of_day(text: &str) -> Option<Timestamp> {
    let mut parts = text.trim().split('-');
    let (y, m, d): (i64, i64, i64) =
        (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
    if parts.next().is_some() || !(1970..=9999).contains(&y) || !(1..=12).contains(&m) || d < 1 {
        return None;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days_in_month = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][(m - 1) as usize];
    if d > days_in_month {
        return None;
    }
    // Days since 1970-01-01 (the civil-from-days algorithm, inverted).
    let (yy, mm) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let doy = (153 * mm + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(Timestamp((days + 1) * 86_400_000 - 1))
}

fn parse_id(text: &str) -> Option<MemoryId> {
    text.trim().trim_start_matches("m:").parse().ok()
}

/// The memory tools of an application.
pub struct MemoryTools {
    store: Arc<dyn MemoryStore>,
    repository: Arc<dyn ExecutionRepository>,
    embedder: Option<Arc<Embedder>>,
    clock: Arc<dyn Clock>,
}

impl MemoryTools {
    /// The tools over a store. `repository` is where approvals are looked up, so a memory records who approved it.
    pub fn new(
        store: Arc<dyn MemoryStore>,
        repository: Arc<dyn ExecutionRepository>,
        embedder: Option<Arc<Embedder>>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self { store, repository, embedder, clock }
    }

    /// Every memory tool.
    pub fn tools(&self) -> Vec<Arc<dyn Tool>> {
        vec![
            Arc::new(Remember { tools: self.shared() }),
            Arc::new(Search { tools: self.shared(), spec: search_spec() }),
            Arc::new(History { tools: self.shared(), spec: history_spec() }),
            Arc::new(Knowledge { tools: self.shared(), spec: knowledge_spec() }),
        ]
    }

    /// Add them to a registry.
    pub fn register(&self, registry: &dyn ToolRegistry) -> zorix_core::Result<()> {
        for tool in self.tools() {
            registry.register(tool)?;
        }
        Ok(())
    }

    fn shared(&self) -> Shared {
        Shared {
            store: self.store.clone(),
            repository: self.repository.clone(),
            embedder: self.embedder.clone(),
            clock: self.clock.clone(),
        }
    }
}

#[derive(Clone)]
struct Shared {
    store: Arc<dyn MemoryStore>,
    repository: Arc<dyn ExecutionRepository>,
    embedder: Option<Arc<Embedder>>,
    clock: Arc<dyn Clock>,
}

// ---------------------------------------------------------------------------------------------------------------------

struct Remember {
    tools: Shared,
}

fn remember_spec() -> ToolSpec {
    let mut spec = ToolSpec::new(
        "memory.remember",
        "Remember",
        "Propose a lasting fact or preference to remember in future conversations. A person must approve it, and sees exactly \\
         this text. Use it only for things that stay true and will matter later (a decision, a convention, a preference), never \\
         for secrets or for what was just said. Call memory.search first and pass the ids of anything this replaces or corrects.",
        json!({
            "type": "object",
            "properties": {
                "content": {"type": "string", "minLength": 1, "maxLength": 500, "description": "One short, self-contained statement."},
                "kind": {"type": "string", "enum": ["fact", "preference"]},
                "scope": {"type": "string", "enum": ["global", "project", "conversation"], "description": "global: always. project: only for runs in this workspace folder. conversation: only this conversation."},
                "subject": {"type": "string", "maxLength": 80, "description": "What it is about, such as 'database' or 'language'. A newer item about the same subject replaces the older one."},
                "valid_until": {"type": "string", "description": "A date (YYYY-MM-DD) after which it should be forgotten."},
                "replaces": {"type": "array", "maxItems": 10, "items": {"type": "string"}, "description": "Ids (m:…) of memories this one replaces."}
            },
            "required": ["content", "kind", "scope"],
            "additionalProperties": false
        }),
    );
    spec.annotations = ToolAnnotations { read_only: false, destructive: false, idempotent: true, open_world: false };
    spec.requires_approval = true;
    spec.approval_by_person_only = true;
    spec
}

#[async_trait]
impl Tool for Remember {
    fn spec(&self) -> &ToolSpec {
        static SPEC: std::sync::OnceLock<ToolSpec> = std::sync::OnceLock::new();
        SPEC.get_or_init(remember_spec)
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let t = &self.tools;
        let content = args["content"].as_str().unwrap_or("").trim().to_string();
        if content.is_empty() {
            return Err(failed("There is nothing to remember."));
        }
        if redact_text(&content) != content {
            return Err(failed(
                "That looks like a credential or a key, so it was not remembered. Memory is not for secrets.",
            ));
        }
        let kind = MemoryKind::parse(args["kind"].as_str().unwrap_or(""))
            .ok_or_else(|| failed("The kind is fact or preference."))?;
        let vis = visibility(ctx);
        let scope = match args["scope"].as_str().unwrap_or("") {
            "global" => Scope::Global,
            "project" => Scope::Project(vis.project.clone().ok_or_else(|| failed("This run has no workspace folder, so there is no project to remember it for. Use global or conversation."))?),
            "conversation" => Scope::Conversation(vis.conversation.clone().ok_or_else(|| failed("This run is not part of a conversation. Use global or project."))?),
            _ => return Err(failed("The scope is global, project or conversation.")),
        };
        let now = t.clock.now();
        let valid_until = match args["valid_until"].as_str().filter(|s| !s.trim().is_empty()) {
            Some(text) => {
                let at =
                    end_of_day(text).ok_or_else(|| failed("valid_until must be a real date written YYYY-MM-DD."))?;
                if at.millis() <= now.millis() {
                    return Err(failed("valid_until is already in the past."));
                }
                Some(at)
            }
            None => None,
        };
        let mut replaces = Vec::new();
        for text in args["replaces"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            let id = parse_id(text).ok_or_else(|| failed(format!("“{text}” is not a memory id (m:…).")))?;
            let item = t.store.memory(&ctx.tenant, id).await.map_err(|e| failed(e.to_string()))?;
            match item {
                Some(item) if vis.sees(&item.scope) => replaces.push(id),
                _ => return Err(failed(format!("“{text}” is not a memory this run can replace."))),
            }
        }
        // How many this execution already proposed, and who approved this one.
        let calls = t.repository.tool_calls(ctx.execution_id).await.map_err(|e| failed(e.to_string()))?;
        let done =
            calls.iter().filter(|c| c.tool == "memory.remember" && c.status == ToolCallStatus::Succeeded).count();
        if done >= MAX_PROPOSALS_PER_EXECUTION {
            return Err(failed("This run has already proposed ten memories. Ask the person to continue in a new run."));
        }
        let approval = t
            .repository
            .approvals(ctx.execution_id)
            .await
            .map_err(|e| failed(e.to_string()))?
            .into_iter()
            .find(|a| a.call_id == ctx.call_id && a.status == ApprovalStatus::Approved)
            .ok_or_else(|| failed("There is no approval for this call, so nothing was remembered."))?;
        let origin = MemoryOrigin::Model {
            execution_id: ctx.execution_id,
            call_id: ctx.call_id.clone(),
            approval_id: Some(approval.id),
            approved_by: approval.decided_by.clone().unwrap_or_default(),
        };
        let new = NewMemory {
            scope,
            kind,
            subject: args["subject"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string),
            content: content.clone(),
            valid_until,
            origin,
            replaces,
        };
        let outcome = t.store.remember(&ctx.tenant, new, now).await.map_err(|e| failed(e.to_string()))?;
        // Best effort: give it a vector now, so it can be found by meaning.
        if let (Some(embedder), Ok(settings)) = (&t.embedder, t.store.settings(&ctx.tenant).await)
            && let Some(model) = settings.embedding_model
        {
            let hash = zorix_core::text_hash(content.trim());
            let _ = embedder
                .fill(
                    &*t.store,
                    &model,
                    settings.embedding_dimensions,
                    vec![(hash, content.trim().to_string())],
                    &ctx.cancel,
                )
                .await;
        }
        let item = outcome.item();
        Ok(match &outcome {
            Remembered::Added(_) => json!({"remembered": true, "status": "added", "id": format!("m:{}", item.id)}),
            Remembered::Duplicate(_) => {
                json!({"remembered": true, "status": "already_remembered", "id": format!("m:{}", item.id)})
            }
            Remembered::Replaced { old, .. } => {
                json!({"remembered": true, "status": "replaced", "id": format!("m:{}", item.id), "replaced": old.iter().map(|i| format!("m:{i}")).collect::<Vec<_>>()})
            }
        })
    }
}

// ---------------------------------------------------------------------------------------------------------------------

struct Search {
    tools: Shared,
    spec: ToolSpec,
}

fn search_spec() -> ToolSpec {
    let mut spec = ToolSpec::new(
        "memory.search",
        "Search memory",
        "Search what is remembered (facts and preferences approved by the user): global ones, this project's and this \\
         conversation's. Returns ids, scope and when each was last confirmed. With no query, lists what is remembered.",
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "maxLength": 500},
                "limit": {"type": "integer", "minimum": 1, "maximum": 10}
            },
            "additionalProperties": false
        }),
    );
    spec.annotations = ToolAnnotations::safe_read();
    spec
}

#[async_trait]
impl Tool for Search {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let t = &self.tools;
        let vis = visibility(ctx);
        let limit = args["limit"].as_u64().unwrap_or(5).clamp(1, MAX_RESULTS as u64) as usize;
        let query = args["query"].as_str().unwrap_or("").trim().to_string();
        let now = t.clock.now();
        let mut degraded = None;
        let items = if query.is_empty() {
            t.store
                .list_memory(&vis, &MemoryFilter { limit, ..MemoryFilter::default() }, now)
                .await
                .map_err(|e| failed(e.to_string()))?
        } else {
            let found = hybrid_search(&*t.store, t.embedder.as_deref(), &vis, zorix_core::Corpus::Memory, &query)
                .await
                .map_err(|e| failed(e.to_string()))?;
            degraded = found.degraded;
            let mut items = Vec::new();
            for hit in found.hits.iter().take(limit) {
                if let Some(id) = parse_id(&hit.id)
                    && let Ok(Some(item)) = t.store.memory(&ctx.tenant, id).await
                    && item.in_force(now)
                {
                    items.push(item);
                }
            }
            items
        };
        let results: Vec<Value> = items
            .iter()
            .map(|i| {
                json!({
                    "id": format!("m:{}", i.id),
                    "kind": i.kind.as_str(),
                    "scope": i.scope.label(),
                    "subject": i.subject,
                    "content": cut(&i.content, 500),
                    "confirmed": date(i.confirmed_at),
                    "valid_until": i.valid_until.map(date),
                })
            })
            .collect();
        Ok(json!({"results": results, "count": results.len(), "note": degraded}))
    }
}

// ---------------------------------------------------------------------------------------------------------------------

struct History {
    tools: Shared,
    spec: ToolSpec,
}

fn history_spec() -> ToolSpec {
    let mut spec = ToolSpec::new(
        "history.search",
        "Search past work",
        "Search records of what was done in earlier executions of this project (what was asked, what happened, which tools ran, \
         who approved what). Use it to avoid repeating work or to learn what was decided before.",
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "minLength": 1, "maxLength": 500},
                "limit": {"type": "integer", "minimum": 1, "maximum": 10}
            },
            "required": ["query"],
            "additionalProperties": false
        }),
    );
    spec.annotations = ToolAnnotations::safe_read();
    spec
}

#[async_trait]
impl Tool for History {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let t = &self.tools;
        let vis = visibility(ctx);
        let limit = args["limit"].as_u64().unwrap_or(5).clamp(1, MAX_RESULTS as u64) as usize;
        let query = args["query"].as_str().unwrap_or("").trim().to_string();
        if query.is_empty() {
            return Err(failed("Say what to look for."));
        }
        let found = hybrid_search(&*t.store, t.embedder.as_deref(), &vis, zorix_core::Corpus::Operations, &query)
            .await
            .map_err(|e| failed(e.to_string()))?;
        let mut results = Vec::new();
        for hit in found.hits.iter().take(limit + 1) {
            let Ok(id) = hit.id.parse::<ExecutionId>() else { continue };
            if id == ctx.execution_id {
                continue;
            }
            if let Ok(Some(record)) = t.store.operation(&ctx.tenant, id).await {
                results.push(json!({
                    "execution": id.to_string(),
                    "state": record.state,
                    "finished": date(record.finished_at),
                    "summary": cut(&record.text, 600),
                }));
            }
            if results.len() == limit {
                break;
            }
        }
        Ok(json!({"results": results, "count": results.len(), "note": found.degraded}))
    }
}

// ---------------------------------------------------------------------------------------------------------------------

struct Knowledge {
    tools: Shared,
    spec: ToolSpec,
}

fn knowledge_spec() -> ToolSpec {
    let mut spec = ToolSpec::new(
        "knowledge.search",
        "Search documents",
        "Search the documents the user has indexed (this project's and the shared ones). Returns passages with their source, \
         such as guide/deploy.md:L10-L42 or manual.pdf#page=3. Cite the source when you rely on a passage.",
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "minLength": 1, "maxLength": 500},
                "limit": {"type": "integer", "minimum": 1, "maximum": 10}
            },
            "required": ["query"],
            "additionalProperties": false
        }),
    );
    spec.annotations = ToolAnnotations::safe_read();
    spec
}

#[async_trait]
impl Tool for Knowledge {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let t = &self.tools;
        let vis = visibility(ctx);
        let limit = args["limit"].as_u64().unwrap_or(5).clamp(1, MAX_RESULTS as u64) as usize;
        let query = args["query"].as_str().unwrap_or("").trim().to_string();
        if query.is_empty() {
            return Err(failed("Say what to look for."));
        }
        let found = hybrid_search(&*t.store, t.embedder.as_deref(), &vis, zorix_core::Corpus::Knowledge, &query)
            .await
            .map_err(|e| failed(e.to_string()))?;
        let ids: Vec<i64> = found.hits.iter().take(limit).filter_map(|h| h.id.parse().ok()).collect();
        let chunks = t.store.chunks(&ctx.tenant, &ids).await.map_err(|e| failed(e.to_string()))?;
        let mut results = Vec::new();
        for id in &ids {
            if let Some(chunk) = chunks.iter().find(|c| c.id == *id) {
                results.push(json!({
                    "source": chunk.citation(),
                    "folder": chunk.root_path,
                    "heading": chunk.heading,
                    "text": cut(&redact_text(&chunk.text), 900),
                }));
            }
        }
        Ok(json!({"results": results, "count": results.len(), "note": found.degraded}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_becomes_the_end_of_that_day_and_nonsense_is_refused() {
        assert_eq!(end_of_day("1970-01-01"), Some(Timestamp(86_400_000 - 1)));
        assert_eq!(end_of_day("2026-11-16"), Some(Timestamp(1_794_873_599_999)));
        assert!(end_of_day("2028-02-29").is_some(), "a leap day");
        for bad in [
            "2027-02-29",
            "2026-13-01",
            "2026-00-10",
            "2026-11-31",
            "11/16/2026",
            "2026-11",
            "",
            "2026-11-16-1",
            "abcd-ef-gh",
            "1969-12-31",
        ] {
            assert_eq!(end_of_day(bad), None, "{bad}");
        }
    }

    #[test]
    fn ids_are_read_with_or_without_the_prefix() {
        let id = MemoryId::new();
        assert_eq!(parse_id(&format!("m:{id}")), Some(id));
        assert_eq!(parse_id(&format!(" {id} ")), Some(id));
        assert_eq!(parse_id("nope"), None);
    }

    #[test]
    fn remembering_needs_a_person_and_searching_does_not() {
        let spec = remember_spec();
        assert!(
            spec.requires_approval
                && spec.approval_by_person_only
                && spec.annotations.idempotent
                && !spec.annotations.read_only
        );
        let search = search_spec();
        assert!(!search.requires_approval && search.annotations.read_only);
    }
}
