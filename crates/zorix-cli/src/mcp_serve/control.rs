//! Following, answering and stopping runs over MCP, and ZORIX's read-only resources (ADR-0052).
//!
//! A client is not the person. It sees the runs this server started, answers an approval only when the person allowed that and
//! then said yes to that exact call, and stops only runs it started itself.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use zorix_core::{
    ApprovalDecision, ApprovalId, ExecutionEngine, ExecutionId, ExecutionState, ListFilter, TenantId, Tool,
    ToolAnnotations, ToolContext, ToolError, ToolSpec, TreeApproval,
};
use zorix_mcp::{Confirmation, ResourceInfo, Resources};
use zorix_store::SqliteAgentStore;

use super::{MAX_WAIT_SECONDS, RunShared, failed, parse_id};

/// The most runs one listing names.
const MAX_RUNS: usize = 50;
/// The most approvals one listing names.
const MAX_APPROVALS: usize = 20;
/// The most of a call's arguments shown to the person, in characters. A longer call is answered in ZORIX: nobody can agree to
/// what they were not shown.
const MAX_SHOWN: usize = 4_000;
/// The most text of a result a resource carries, in bytes.
const MAX_RESOURCE_TEXT: usize = 32 * 1024;

/// Text for a person, with the characters that could hide or reorder it written out.
fn plain(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\n' => "\n".to_string(),
            '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}' => {
                format!("\\u{{{:x}}}", c as u32)
            }
            c if c.is_control() => format!("\\u{{{:x}}}", c as u32),
            c => c.to_string(),
        })
        .collect()
}

fn cut(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

impl RunShared {
    /// Whether this server started the run, or the run at the top of its tree.
    async fn started_here(&self, id: ExecutionId) -> bool {
        if self.started.lock().contains(&id) {
            return true;
        }
        let root = self.app.engine.get(&self.tenant, id).await.ok().and_then(|e| e.root);
        root.is_some_and(|root| self.started.lock().contains(&root))
    }

    /// The runs a client may see, newest first.
    async fn visible_runs(
        &self,
        states: Vec<ExecutionState>,
        limit: usize,
    ) -> Result<Vec<zorix_core::Execution>, ToolError> {
        let engine = &self.app.engine;
        if self.read_all_runs {
            let filter = ListFilter { states, roots_only: true, limit, ..ListFilter::default() };
            return engine.list(&self.tenant, &filter).await.map_err(|e| failed(e.to_string()));
        }
        let own: Vec<ExecutionId> = self.started.lock().iter().copied().collect();
        let mut runs = Vec::new();
        for id in own {
            let run = engine.get(&self.tenant, id).await.map_err(|e| failed(e.to_string()))?;
            if states.is_empty() || states.contains(&run.state) {
                runs.push(run);
            }
        }
        runs.sort_by_key(|e| std::cmp::Reverse((e.created_at, e.id)));
        runs.truncate(limit);
        Ok(runs)
    }

    /// The approval a call names, which must be waiting in the tree of a run the client may see, with the very hash it was
    /// listed with.
    async fn waiting(&self, args: &Value) -> Result<TreeApproval, ToolError> {
        let id = parse_id(args)?;
        self.may_follow(id).await?;
        let approval: ApprovalId = args["approval_id"]
            .as_str()
            .and_then(|text| text.parse().ok())
            .ok_or_else(|| failed("approval_id is not an approval id."))?;
        let item = self
            .app
            .engine
            .pending_in_tree(&self.tenant, id)
            .await
            .map_err(|e| failed(e.to_string()))?
            .into_iter()
            .find(|w| w.approval.id == approval)
            .ok_or_else(|| failed("No such approval is waiting. List them with zorix.approvals."))?;
        if args["call_hash"].as_str() != Some(item.approval.call_hash.as_str()) {
            return Err(failed("call_hash is not that of the waiting call. List it again with zorix.approvals."));
        }
        Ok(item)
    }
}

// ---------------------------------------------------------------------------------------------------------------------

pub(super) struct RunsTool {
    shared: Arc<RunShared>,
    spec: ToolSpec,
}

impl RunsTool {
    pub(super) fn new(shared: Arc<RunShared>) -> Self {
        let mut spec = ToolSpec::new(
            "zorix.runs",
            "List ZORIX runs",
            "The runs this server started (or, if the person allowed it, the newest runs of ZORIX), with where each stands.",
            json!({
                "type": "object",
                "properties": {"limit": {"type": "integer", "minimum": 1, "maximum": MAX_RUNS}},
                "additionalProperties": false
            }),
        );
        spec.annotations = ToolAnnotations::safe_read();
        Self { shared, spec }
    }
}

#[async_trait]
impl Tool for RunsTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, _ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let limit = args["limit"].as_u64().map_or(20, |n| n as usize).min(MAX_RUNS);
        let runs: Vec<Value> = self
            .shared
            .visible_runs(Vec::new(), limit)
            .await?
            .iter()
            .map(|e| {
                json!({
                    "execution_id": e.id.to_string(),
                    "state": e.state.as_str(),
                    "prompt": cut(&e.request.prompt, 200),
                    "agent": e.agent.as_ref().map(|a| a.slug.to_string()),
                    "tokens": e.usage.total_tokens(),
                    "created_at": e.created_at.0,
                })
            })
            .collect();
        Ok(json!({ "runs": runs }))
    }
}

// ---------------------------------------------------------------------------------------------------------------------

pub(super) struct ApprovalsTool {
    shared: Arc<RunShared>,
    spec: ToolSpec,
}

impl ApprovalsTool {
    pub(super) fn new(shared: Arc<RunShared>) -> Self {
        let mut spec = ToolSpec::new(
            "zorix.approvals",
            "What waits for approval",
            "The tool calls that wait for a person, in the runs this server may show: the tool, its exact arguments, why it \
             needs a person, and the call_hash that zorix.answer_approval asks for.",
            json!({"type": "object", "properties": {}, "additionalProperties": false}),
        );
        spec.annotations = ToolAnnotations::safe_read();
        Self { shared, spec }
    }
}

#[async_trait]
impl Tool for ApprovalsTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, _ctx: &ToolContext, _args: Value) -> Result<Value, ToolError> {
        let shared = &self.shared;
        let mut approvals = Vec::new();
        // A run waits itself or has a child that does, so every visible run is looked into, not only the waiting ones.
        'runs: for run in shared.visible_runs(Vec::new(), MAX_RUNS).await? {
            if run.state.is_terminal() {
                continue;
            }
            let waiting =
                shared.app.engine.pending_in_tree(&shared.tenant, run.id).await.map_err(|e| failed(e.to_string()))?;
            for item in waiting {
                if approvals.len() == MAX_APPROVALS {
                    break 'runs;
                }
                let arguments = item.approval.arguments.to_string();
                let mut entry = json!({
                    "execution_id": run.id.to_string(),
                    "approval_id": item.approval.id.to_string(),
                    "call_hash": item.approval.call_hash,
                    "tool": item.approval.tool,
                    "reason": item.approval.reason,
                    "asked_by": item.agent,
                    "depth": item.depth,
                });
                if arguments.len() <= MAX_SHOWN {
                    entry["arguments"] = item.approval.arguments.clone();
                } else {
                    entry["arguments_omitted"] = json!("Too long to show here. Answer this one in ZORIX.");
                }
                approvals.push(entry);
            }
        }
        Ok(json!({ "approvals": approvals }))
    }
}

// ---------------------------------------------------------------------------------------------------------------------

/// What the person is asked before an answer is given in their name.
pub(super) struct AnswerQuestion(pub(super) Arc<RunShared>);

#[async_trait]
impl Confirmation for AnswerQuestion {
    async fn question(&self, args: &Value) -> Result<String, String> {
        let shared = &self.0;
        let reason = |e: ToolError| match e {
            ToolError::Failed(message) | ToolError::Retryable(message) => message,
        };
        let item = shared.waiting(args).await.map_err(reason)?;
        let approve = args["approve"].as_bool().unwrap_or(false);
        let tool = &item.approval.tool;
        let mut arguments = pretty(&item.approval.arguments);
        if approve {
            // Fail closed: a tool this process does not know cannot be checked, nor run.
            let spec = shared.app.engine.tools().get(tool).map(|t| t.spec().approval_by_person_only);
            if spec.is_none_or(|person_only| person_only) {
                return Err(format!(
                    "{tool} can only be approved in ZORIX itself, where you see all of what it does. Nothing was done."
                ));
            }
            if arguments.chars().count() > MAX_SHOWN {
                return Err(
                    "This call is too long to show here whole, so it is answered in ZORIX. Nothing was done.".into()
                );
            }
        } else {
            arguments = cut(&arguments, MAX_SHOWN);
        }
        let asked_by = item.agent.as_deref().map(|a| format!(" (asked by {a})")).unwrap_or_default();
        Ok(format!(
            "ZORIX: {verb} this call?\n\nTool: {tool}{asked_by}\nRun: {run}\nWhy it needs you: {why}\nArguments:\n{arguments}",
            verb = if approve { "ALLOW" } else { "REFUSE" },
            tool = plain(tool),
            run = item.execution,
            why = plain(&item.approval.reason).replace('\n', " "),
            arguments = plain(&arguments),
        ))
    }
}

pub(super) struct AnswerTool {
    shared: Arc<RunShared>,
    spec: ToolSpec,
}

impl AnswerTool {
    pub(super) fn new(shared: Arc<RunShared>) -> Self {
        let mut spec = ToolSpec::new(
            "zorix.answer_approval",
            "Answer an approval",
            "Allow or refuse a tool call listed by zorix.approvals. The person is asked to confirm every answer, with the \
             exact call in front of them; without their yes nothing happens. Then waits for the run for a while.",
            json!({
                "type": "object",
                "properties": {
                    "execution_id": {"type": "string", "minLength": 1, "maxLength": 100},
                    "approval_id": {"type": "string", "minLength": 1, "maxLength": 100},
                    "call_hash": {"type": "string", "minLength": 1, "maxLength": 200},
                    "approve": {"type": "boolean"},
                    "note": {"type": "string", "maxLength": 2000},
                    "wait_seconds": {"type": "integer", "minimum": 0, "maximum": MAX_WAIT_SECONDS}
                },
                "required": ["execution_id", "approval_id", "call_hash", "approve"],
                "additionalProperties": false
            }),
        );
        spec.annotations = ToolAnnotations { read_only: false, destructive: true, idempotent: false, open_world: true };
        spec.timeout_ms = (MAX_WAIT_SECONDS + 30) * 1000;
        Self { shared, spec }
    }
}

#[async_trait]
impl Tool for AnswerTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let shared = &self.shared;
        let id = parse_id(&args)?;
        let item = shared.waiting(&args).await?;
        let wait = args["wait_seconds"].as_u64().unwrap_or(30).min(MAX_WAIT_SECONDS);
        let decision = ApprovalDecision {
            approve: args["approve"].as_bool().unwrap_or(false),
            decided_by: "mcp client, confirmed by the person".into(),
            note: args["note"].as_str().map(str::to_string),
            call_hash: Some(item.approval.call_hash.clone()),
            automatic: false,
        };
        let engine = &shared.app.engine;
        engine
            .decide(&shared.tenant, item.execution, item.approval.id, decision)
            .await
            .map_err(|e| failed(e.to_string()))?;
        // A client that stops waiting does not stop the run: the answer was given.
        tokio::select! {
            _ = ctx.cancel.cancelled() => {}
            _ = tokio::time::timeout(Duration::from_secs(wait), engine.settle(&shared.tenant, id)) => {}
        }
        shared.snapshot(id, true).await
    }
}

// ---------------------------------------------------------------------------------------------------------------------

pub(super) struct CancelTool {
    shared: Arc<RunShared>,
    spec: ToolSpec,
}

impl CancelTool {
    pub(super) fn new(shared: Arc<RunShared>) -> Self {
        let mut spec = ToolSpec::new(
            "zorix.cancel",
            "Stop a run",
            "Stop a run this server started. Runs started elsewhere are stopped in ZORIX.",
            super::id_schema(),
        );
        spec.annotations = ToolAnnotations { read_only: false, destructive: true, idempotent: true, open_world: false };
        Self { shared, spec }
    }
}

#[async_trait]
impl Tool for CancelTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, _ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let id = parse_id(&args)?;
        if !self.shared.started_here(id).await {
            return Err(failed("That execution was not started by this server, so it cannot be stopped through it."));
        }
        self.shared
            .app
            .engine
            .cancel(&self.shared.tenant, id, Some("an MCP client asked".into()))
            .await
            .map_err(|e| failed(e.to_string()))?;
        self.shared.snapshot(id, false).await
    }
}

// ---------------------------------------------------------------------------------------------------------------------

/// `zorix://agents/{name}` and, when runs are offered, `zorix://runs/{id}`.
pub(super) struct ZorixResources {
    pub(super) agents: Arc<SqliteAgentStore>,
    pub(super) tenant: TenantId,
    pub(super) runs: Option<Arc<RunShared>>,
}

#[async_trait]
impl Resources for ZorixResources {
    async fn list(&self) -> Vec<ResourceInfo> {
        let mut out: Vec<ResourceInfo> = zorix_agents::cards(&*self.agents, &self.tenant, false)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|c| ResourceInfo {
                uri: format!("zorix://agents/{}", c.slug),
                name: c.name,
                description: c.description,
            })
            .collect();
        if let Some(runs) = &self.runs {
            for run in runs.visible_runs(Vec::new(), MAX_RUNS).await.unwrap_or_default() {
                out.push(ResourceInfo {
                    uri: format!("zorix://runs/{}", run.id),
                    name: cut(&run.request.prompt, 80),
                    description: run.state.as_str().to_string(),
                });
            }
        }
        out
    }

    async fn read(&self, uri: &str) -> Option<String> {
        // The address is an input like any other: only a name or an id that exists, never anything read as a path.
        if let Some(name) = uri.strip_prefix("zorix://agents/") {
            let card = zorix_agents::cards(&*self.agents, &self.tenant, false)
                .await
                .ok()?
                .into_iter()
                .find(|c| c.slug == name)?;
            return Some(pretty(&json!({
                "agent": card.slug, "name": card.name, "description": card.description,
                "default_model": card.default_model, "version": card.version, "skills": card.skills,
            })));
        }
        let id: ExecutionId = uri.strip_prefix("zorix://runs/")?.parse().ok()?;
        let mut snapshot = self.runs.as_ref()?.snapshot(id, true).await.ok()?;
        if let Some(text) = snapshot["result"].as_str()
            && text.len() > MAX_RESOURCE_TEXT
        {
            let mut end = MAX_RESOURCE_TEXT;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            snapshot["result"] = json!(format!("{}\n[ZORIX cut the rest of this result]", &text[..end]));
        }
        if snapshot["structured"].to_string().len() > MAX_RESOURCE_TEXT {
            snapshot["structured"] = json!("too long to include");
        }
        Some(pretty(&snapshot))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_and_reordering_characters_are_written_out() {
        assert_eq!(plain("a\u{202e}b\u{200b}c\r\u{1b}d\ne"), "a\\u{202e}b\\u{200b}c\\u{d}\\u{1b}d\ne");
        assert_eq!(cut("ééé", 2), "éé…");
        assert_eq!(cut("ok", 2), "ok");
    }
}
