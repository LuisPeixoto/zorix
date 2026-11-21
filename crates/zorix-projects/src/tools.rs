//! The tools agents use with projects (ADR-0037).
//!
//! A manager gets `tasks.*` with `project:KEY[:write]`: every write asks a person, and a move asks a person who is looking (no
//! automatic approval). A run a task started gets `task.*` with `task:KEY`: it can report progress, add what it produced and leave
//! a note on that one task, and nothing else. Tools hold the service weakly, because the service holds the engine that holds them.

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use serde_json::{Value, json};
use zorix_core::{
    Action, AgentSlug, Date, DeliverableKind, Permission, ProjectKey, TaskKey, TaskPriority, TaskState, Tool,
    ToolAnnotations, ToolContext, ToolError, ToolSource, ToolSpec, Who,
};
use zorix_workflows::host::Ticker;

use crate::service::{Move, NewTask, ProjectService, TaskFields, TaskView};

fn failed(message: impl Into<String>) -> ToolError {
    ToolError::Failed(message.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    List,
    Get,
    Create,
    Update,
    Move,
    Comment,
    ReportProgress,
    AddDeliverable,
    Note,
}

impl Op {
    const ALL: [Op; 9] = [
        Op::List,
        Op::Get,
        Op::Create,
        Op::Update,
        Op::Move,
        Op::Comment,
        Op::ReportProgress,
        Op::AddDeliverable,
        Op::Note,
    ];

    fn id(self) -> &'static str {
        match self {
            Op::List => "tasks.list",
            Op::Get => "tasks.get",
            Op::Create => "tasks.create",
            Op::Update => "tasks.update",
            Op::Move => "tasks.move",
            Op::Comment => "tasks.comment",
            Op::ReportProgress => "task.report_progress",
            Op::AddDeliverable => "task.add_deliverable",
            Op::Note => "task.note",
        }
    }

    /// The manager's tools change a project and ask first. The tools of a task's own run change only that task.
    fn manager_write(self) -> bool {
        matches!(self, Op::Create | Op::Update | Op::Move | Op::Comment)
    }
}

/// One project tool.
pub struct ProjectTool {
    service: Weak<ProjectService>,
    op: Op,
    spec: ToolSpec,
}

fn schema(op: Op) -> (&'static str, Value, Vec<&'static str>) {
    let project =
        json!({"type": "string", "pattern": "^[A-Z][A-Z0-9]{1,4}$", "description": "The project key, such as OPS."});
    let task = json!({"type": "string", "pattern": "^[A-Z][A-Z0-9]{1,4}-[0-9]+$", "description": "The task key, such as OPS-38."});
    let assignee =
        json!({"type": "string", "maxLength": 60, "description": "`you`, `agent:SLUG`, `team:SLUG` or `none`."});
    let day = |what: &str| json!({"type": "string", "maxLength": 10, "description": format!("{what} as YYYY-MM-DD, or `none` to clear it.")});
    let priority = json!({"type": "string", "enum": ["urgent", "high", "normal", "low"]});
    let state = json!({"type": "string", "enum": ["backlog", "in_progress", "review", "done"]});
    match op {
        Op::List => (
            "List the tasks of a project: key, title, state, priority, who has it, progress, dates and what it waits for.",
            json!({"project": project, "state": state}),
            vec!["project"],
        ),
        Op::Get => (
            "Read one task: its description, links, deliverables (titles), history of runs and what it waits for.",
            json!({"task": task}),
            vec!["task"],
        ),
        Op::Create => (
            "Make a task in a project. A person approves it first.",
            json!({
                "project": project,
                "title": {"type": "string", "minLength": 1, "maxLength": 200},
                "description": {"type": "string", "maxLength": 20000},
                "priority": priority,
                "assignee": assignee,
                "start": day("The first day"),
                "due": day("The last day"),
                "depends_on": {"type": "array", "items": task.clone(), "maxItems": 20},
            }),
            vec!["project", "title"],
        ),
        Op::Update => (
            "Change a task's title, description, priority, assignee, dates or progress. Only what is given changes. A person approves it first.",
            json!({
                "task": task,
                "title": {"type": "string", "minLength": 1, "maxLength": 200},
                "description": {"type": "string", "maxLength": 20000},
                "priority": priority,
                "assignee": assignee,
                "start": day("The first day"),
                "due": day("The last day"),
                "progress": {"type": "integer", "minimum": 0, "maximum": 100},
            }),
            vec!["task"],
        ),
        Op::Move => (
            "Move a task to a lane. Moving a task that an agent or a team has to In progress starts its run. A person who is looking approves every move, and a move to done is a person's call.",
            json!({
                "task": task,
                "to": state,
                "note": {"type": "string", "maxLength": 2000, "description": "When sending a task back: what to change."},
            }),
            vec!["task", "to"],
        ),
        Op::Comment => (
            "Leave a comment on a task. A person approves it first.",
            json!({"task": task, "text": {"type": "string", "minLength": 1, "maxLength": 4000}}),
            vec!["task", "text"],
        ),
        Op::ReportProgress => (
            "Say how far the task you were given is, from 0 to 100.",
            json!({"task": task, "percent": {"type": "integer", "minimum": 0, "maximum": 100}}),
            vec!["task", "percent"],
        ),
        Op::AddDeliverable => (
            "Put what you produced on your task: an answer (text) or a link. A person reviews it.",
            json!({
                "task": task,
                "kind": {"type": "string", "enum": ["answer", "link"]},
                "title": {"type": "string", "minLength": 1, "maxLength": 200},
                "body": {"type": "string", "minLength": 1, "maxLength": 200000, "description": "The text, or the address of the link."},
            }),
            vec!["task", "kind", "title", "body"],
        ),
        Op::Note => (
            "Leave a note on your task for the person who reviews it.",
            json!({"task": task, "text": {"type": "string", "minLength": 1, "maxLength": 4000}}),
            vec!["task", "text"],
        ),
    }
}

/// The tools of projects. They are only offered to runs that were granted them.
pub fn project_tools(service: Weak<ProjectService>) -> Vec<Arc<dyn Tool>> {
    Op::ALL
        .into_iter()
        .map(|op| {
            let (describe, properties, required) = schema(op);
            let mut spec = ToolSpec::new(
                op.id(),
                op.id().replace(['.', '_'], " "),
                describe,
                json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false}),
            );
            spec.source = ToolSource::Builtin;
            spec.timeout_ms = 30_000;
            spec.requires_approval = op.manager_write();
            spec.approval_by_person_only = op == Op::Move;
            spec.annotations = if matches!(op, Op::List | Op::Get) {
                ToolAnnotations::safe_read()
            } else {
                ToolAnnotations { read_only: false, destructive: false, idempotent: op == Op::ReportProgress, open_world: false }
            };
            Arc::new(ProjectTool { service: service.clone(), op, spec }) as Arc<dyn Tool>
        })
        .collect()
}

fn text<'a>(args: &'a Value, name: &str) -> Option<&'a str> {
    args.get(name).and_then(Value::as_str)
}

fn task_key(args: &Value) -> Result<TaskKey, ToolError> {
    text(args, "task").ok_or_else(|| failed("task is missing."))?.parse().map_err(failed)
}

fn project_key(args: &Value) -> Result<ProjectKey, ToolError> {
    ProjectKey::new(text(args, "project").ok_or_else(|| failed("project is missing."))?).map_err(failed)
}

fn who(text: &str) -> Result<Option<Who>, ToolError> {
    let bad = || failed("An assignee is `you`, `agent:SLUG`, `team:SLUG` or `none`.");
    Ok(match text {
        "none" | "" => None,
        "you" => Some(Who::You),
        other => {
            let (kind, slug) = other.split_once(':').ok_or_else(bad)?;
            let slug = AgentSlug::new(slug).map_err(failed)?;
            match kind {
                "agent" => Some(Who::Agent { slug }),
                "team" => Some(Who::Team { slug }),
                _ => return Err(bad()),
            }
        }
    })
}

fn day(text: &str) -> Result<Option<Date>, ToolError> {
    if text == "none" || text.is_empty() {
        return Ok(None);
    }
    Date::new(text).map(Some).map_err(failed)
}

fn priority(text: &str) -> Result<TaskPriority, ToolError> {
    text.parse().map_err(failed)
}

fn summary(view: &TaskView) -> Value {
    let task = &view.task;
    json!({
        "key": task.key.to_string(),
        "title": task.title,
        "state": task.state.as_str(),
        "priority": serde_json::to_value(task.priority).unwrap_or(Value::Null),
        "assignee": task.assignee.as_ref().map(Who::label),
        "progress": task.progress,
        "start": task.start,
        "due": task.due,
        "waits_for": view.blocked_by.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "needs_a_person": view.needs_you.map(|n| serde_json::to_value(n).unwrap_or(Value::Null)),
    })
}

impl ProjectTool {
    fn service(&self) -> Result<Arc<ProjectService>, ToolError> {
        self.service.upgrade().ok_or_else(|| failed("Projects are not available right now."))
    }

    fn actor(ctx: &ToolContext) -> Result<Option<Who>, ToolError> {
        Ok(ctx.agent.as_ref().map(|a| Who::Agent { slug: a.slug.clone() }))
    }

    fn allow(ctx: &ToolContext, action: Action, resource: String, what: &str) -> Result<(), ToolError> {
        if ctx.grants.allows(&Permission::new(action, resource.clone())) {
            Ok(())
        } else {
            Err(failed(format!("This run has no permission for {what} ({resource}).")))
        }
    }
}

#[async_trait]
impl Tool for ProjectTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn describe_call(&self, args: &Value) -> Option<String> {
        let task = text(args, "task");
        Some(match self.op {
            Op::Create => format!("Create a task “{}” in {}", text(args, "title")?, text(args, "project")?),
            Op::Update => format!("Change {}", task?),
            Op::Move => format!("Move {} to {}", task?, text(args, "to")?.replace('_', " ")),
            Op::Comment => format!("Comment on {}", task?),
            _ => return None,
        })
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let service = self.service()?;
        let by = Self::actor(ctx)?;
        let own = |key: &TaskKey| -> Result<(), ToolError> {
            Self::allow(ctx, Action::Write, format!("task:{key}"), "changing this task")
        };
        let manage = |project: &ProjectKey| -> Result<(), ToolError> {
            Self::allow(ctx, Action::Write, format!("project:{project}"), "changing this project")
        };
        match self.op {
            Op::List => {
                let key = project_key(&args)?;
                Self::allow(ctx, Action::Read, format!("project:{key}"), "reading this project")?;
                let want = text(&args, "state").map(|s| s.parse::<TaskState>()).transpose().map_err(failed)?;
                let tasks = service.tasks(&key).await.map_err(|e| failed(e.to_string()))?;
                let rows: Vec<Value> =
                    tasks.iter().filter(|t| want.is_none_or(|s| t.task.state == s)).map(summary).collect();
                Ok(json!({"project": key.to_string(), "tasks": rows}))
            }
            Op::Get => {
                let key = task_key(&args)?;
                Self::allow(ctx, Action::Read, format!("project:{}", key.project), "reading this project")?;
                let detail = service.task(&key).await.map_err(|e| failed(e.to_string()))?;
                let task = &detail.task.task;
                let mut out = summary(&detail.task);
                out["description"] = json!(task.description);
                out["links"] = json!(task.links.iter().map(|l| l.url.clone()).collect::<Vec<_>>());
                out["deliverables"] = json!(task.deliverables.iter().map(|d| d.title.clone()).collect::<Vec<_>>());
                out["runs"] = json!(task.runs.len());
                Ok(out)
            }
            Op::Create => {
                let key = project_key(&args)?;
                manage(&key)?;
                let new = NewTask {
                    title: text(&args, "title").unwrap_or_default().to_string(),
                    description: text(&args, "description").unwrap_or_default().to_string(),
                    state: None,
                    priority: text(&args, "priority").map(priority).transpose()?.unwrap_or_default(),
                    assignee: text(&args, "assignee").map(who).transpose()?.flatten(),
                    start: text(&args, "start").map(day).transpose()?.flatten(),
                    due: text(&args, "due").map(day).transpose()?.flatten(),
                    depends_on: args
                        .get("depends_on")
                        .and_then(Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(Value::as_str)
                                .map(|s| s.parse::<TaskKey>().map_err(failed))
                                .collect()
                        })
                        .transpose()?
                        .unwrap_or_default(),
                    links: Vec::new(),
                };
                let task = service.add_task(&key, new, by).await.map_err(|e| failed(e.to_string()))?;
                Ok(json!({"created": task.key.to_string()}))
            }
            Op::Update => {
                let key = task_key(&args)?;
                manage(&key.project)?;
                let current = service.task(&key).await.map_err(|e| failed(e.to_string()))?.task.task;
                let mut fields = TaskFields::of(&current);
                if let Some(v) = text(&args, "title") {
                    fields.title = v.to_string();
                }
                if let Some(v) = text(&args, "description") {
                    fields.description = v.to_string();
                }
                if let Some(v) = text(&args, "priority") {
                    fields.priority = priority(v)?;
                }
                if let Some(v) = text(&args, "assignee") {
                    fields.assignee = who(v)?;
                }
                if let Some(v) = text(&args, "start") {
                    fields.start = day(v)?;
                }
                if let Some(v) = text(&args, "due") {
                    fields.due = day(v)?;
                }
                if let Some(v) = args.get("progress").and_then(Value::as_u64) {
                    fields.progress = u8::try_from(v.min(100)).unwrap_or(100);
                }
                service.update_task(&key, fields, by).await.map_err(|e| failed(e.to_string()))?;
                Ok(json!({"updated": key.to_string()}))
            }
            Op::Move => {
                let key = task_key(&args)?;
                manage(&key.project)?;
                let to: TaskState =
                    text(&args, "to").ok_or_else(|| failed("to is missing."))?.parse().map_err(failed)?;
                let mv = Move {
                    to: Some(to),
                    index: None,
                    note: text(&args, "note").map(str::to_string),
                    cancel_run: true,
                    // The call only runs after a person approved it; a move is never approved automatically.
                    approved_by: Some("you".into()),
                };
                let task = service.move_task(&key, mv, by).await.map_err(|e| failed(e.to_string()))?;
                Ok(json!({"moved": key.to_string(), "state": task.state.as_str()}))
            }
            Op::Comment => {
                let key = task_key(&args)?;
                manage(&key.project)?;
                service
                    .comment(&key, text(&args, "text").unwrap_or_default(), by)
                    .await
                    .map_err(|e| failed(e.to_string()))?;
                Ok(json!({"commented": key.to_string()}))
            }
            Op::ReportProgress => {
                let key = task_key(&args)?;
                own(&key)?;
                let percent =
                    args.get("percent").and_then(Value::as_u64).ok_or_else(|| failed("percent is missing."))?;
                let percent = u8::try_from(percent).map_err(|_| failed("percent is from 0 to 100."))?;
                service.report_progress(&key, percent, by).await.map_err(|e| failed(e.to_string()))?;
                Ok(json!({"progress": percent}))
            }
            Op::AddDeliverable => {
                let key = task_key(&args)?;
                own(&key)?;
                let kind = match text(&args, "kind") {
                    Some("answer") => DeliverableKind::Answer,
                    Some("link") => DeliverableKind::Link,
                    _ => return Err(failed("kind is `answer` or `link`.")),
                };
                service
                    .add_deliverable(
                        &key,
                        kind,
                        text(&args, "title").unwrap_or_default(),
                        text(&args, "body").unwrap_or_default(),
                        by,
                        Some(ctx.execution_id),
                    )
                    .await
                    .map_err(|e| failed(e.to_string()))?;
                Ok(json!({"added": text(&args, "title")}))
            }
            Op::Note => {
                let key = task_key(&args)?;
                own(&key)?;
                service
                    .comment(&key, text(&args, "text").unwrap_or_default(), by)
                    .await
                    .map_err(|e| failed(e.to_string()))?;
                Ok(json!({"noted": key.to_string()}))
            }
        }
    }
}

/// Keeps tasks moving: at the end of every beat of the workflow host it reads the runs of tasks and starts, finishes or unblocks them.
pub struct ProjectTicker {
    service: Arc<ProjectService>,
}

impl ProjectTicker {
    /// A ticker for a service.
    pub fn new(service: Arc<ProjectService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl Ticker for ProjectTicker {
    async fn tick(&self) {
        if let Err(error) = self.service.reconcile().await {
            tracing::warn!(%error, "tasks could not be brought up to date");
        }
    }
}
