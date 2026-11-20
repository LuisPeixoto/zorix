//! Delegation: an agent asking other agents to do part of the work (ADR-0029 to ADR-0031).
//!
//! The model calls the tool [`TOOL`] with a list of tasks. The runtime suspends the caller, runs one execution per task, and gives the
//! caller one [`ChildResult`] per task when they have all finished.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::execution::{Execution, NewExecution};
use crate::state::ExecutionState;

/// The tool a model calls to delegate.
pub const TOOL: &str = "agents.delegate";
/// Most tasks in one call.
pub const MAX_TASKS: usize = 8;
/// Deepest nesting: the root is 0, its children 1, and so on.
pub const MAX_DEPTH: u32 = 3;
/// Most descendants one root may ever have.
pub const MAX_DESCENDANTS: usize = 32;
/// Longest task text, in characters.
pub const MAX_TASK_CHARS: usize = 20_000;

/// One thing to ask of one agent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DelegateTask {
    /// The agent's slug.
    pub agent: String,
    /// The objective, written so that it can be done without the rest of the conversation.
    pub task: String,
    /// Data that goes with the task.
    #[serde(default)]
    pub input: Option<Value>,
    /// A JSON Schema the answer must satisfy.
    #[serde(default)]
    pub expected_output: Option<Value>,
    /// Notes the caller chose to pass on.
    #[serde(default)]
    pub context: Vec<String>,
    /// Workspace read patterns the agent may use, narrower than or equal to the caller's.
    #[serde(default)]
    pub files: Vec<String>,
}

/// The arguments of [`TOOL`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DelegateRequest {
    /// The tasks, run at the same time.
    pub tasks: Vec<DelegateTask>,
}

impl DelegateRequest {
    /// Read and check the arguments a model sent. The error is a sentence for the model.
    pub fn parse(arguments: &Value) -> Result<Self, String> {
        let request: DelegateRequest =
            serde_json::from_value(arguments.clone()).map_err(|e| format!("The delegation is not valid: {e}."))?;
        if request.tasks.is_empty() {
            return Err("Delegate at least one task.".into());
        }
        if request.tasks.len() > MAX_TASKS {
            return Err(format!("Delegate at most {MAX_TASKS} tasks in one call."));
        }
        for task in &request.tasks {
            if task.agent.trim().is_empty() {
                return Err("Every task names the agent that should do it.".into());
            }
            if task.task.trim().is_empty() {
                return Err(format!("The task for {} is empty.", task.agent));
            }
            if task.task.chars().count() > MAX_TASK_CHARS {
                return Err(format!("The task for {} is too long ({MAX_TASK_CHARS} characters at most).", task.agent));
            }
            if task.context.len() > 16 || task.context.iter().any(|c| c.chars().count() > 2_000) {
                return Err("Pass at most 16 notes of 2000 characters each.".into());
            }
            if task.files.len() > 16 {
                return Err("Name at most 16 file patterns.".into());
            }
        }
        Ok(request)
    }

    /// The input schema of the tool.
    pub fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "tasks": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MAX_TASKS,
                    "items": {
                        "type": "object",
                        "properties": {
                            "agent": {"type": "string", "minLength": 1, "maxLength": 48, "description": "The slug of the agent to ask."},
                            "task": {"type": "string", "minLength": 1, "maxLength": MAX_TASK_CHARS, "description": "What to do, with the goal, the form of the answer and where to look. The agent does not see this conversation."},
                            "input": {"description": "Data that goes with the task (any JSON)."},
                            "expected_output": {"type": "object", "description": "A JSON Schema the answer must satisfy."},
                            "context": {"type": "array", "maxItems": 16, "items": {"type": "string", "maxLength": 2000}, "description": "Notes to pass on."},
                            "files": {"type": "array", "maxItems": 16, "items": {"type": "string", "maxLength": 512}, "description": "Workspace patterns the agent may read."}
                        },
                        "required": ["agent", "task"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["tasks"],
            "additionalProperties": false
        })
    }
}

/// What came back from one task. A task that failed or was cancelled is reported here, not raised.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChildResult {
    /// The agent that was asked.
    pub agent: String,
    /// The version of it that ran, when known.
    pub version: Option<u32>,
    /// What it was asked.
    pub task: String,
    /// Where the child ended: `completed`, `failed` or `cancelled`.
    pub state: String,
    /// Its answer.
    pub answer: Option<String>,
    /// The answer as data, when an expected output was given.
    pub structured: Option<Value>,
    /// Why it did not complete.
    pub error: Option<String>,
    /// Model steps it took.
    pub steps: u32,
    /// Tool calls it made.
    pub tool_calls: u32,
    /// What it cost, in millionths of a dollar.
    pub cost_micros: u64,
    /// Its execution, to look at.
    pub child_id: String,
}

impl ChildResult {
    /// The result of a child that has finished.
    pub fn of(task: &DelegateTask, child: &Execution) -> Self {
        let error = match child.state {
            ExecutionState::Completed => None,
            ExecutionState::Failed => {
                Some(child.failure.as_ref().map_or_else(|| "It failed.".to_string(), |f| f.message.clone()))
            }
            ExecutionState::Cancelled => {
                Some(child.cancel_reason.clone().unwrap_or_else(|| "It was cancelled.".into()))
            }
            other => Some(format!("It had not finished: it was {other}.")),
        };
        Self {
            agent: task.agent.clone(),
            version: child.agent.as_ref().map(|a| a.version),
            task: task.task.clone(),
            state: child.state.as_str().to_string(),
            answer: child.result.as_ref().map(|r| r.text.clone()),
            structured: child.result.as_ref().and_then(|r| r.structured.clone()),
            error,
            steps: child.iteration,
            tool_calls: child.usage.tool_calls,
            cost_micros: child.usage.cost_micros,
            child_id: child.id.to_string(),
        }
    }
}

/// A task with the execution that will carry it out.
#[derive(Clone, Debug)]
pub struct PlannedChild {
    /// The task as the model wrote it.
    pub task: DelegateTask,
    /// The execution to create. Its parent, root, depth, grants, limits and agent are set by the planner.
    pub new: NewExecution,
}

/// What a planner needs to know about where the call comes from.
pub struct PlanContext<'a> {
    /// The execution that is delegating.
    pub parent: &'a Execution,
    /// The call, which names the children's idempotency keys.
    pub call_id: &'a str,
    /// The parent's ancestors, nearest first (empty for a root).
    pub ancestors: Vec<Execution>,
    /// How many descendants the root already has.
    pub descendants: usize,
}

/// Turns the tasks of one delegate call into executions: checks who may be asked, narrows grants, splits the budget and shapes the
/// handoff. Implemented in `zorix-agents`; the runtime only runs what it plans.
#[async_trait]
pub trait DelegationPlanner: Send + Sync {
    /// The executions for these tasks, in the same order, or the reason the delegation is refused (a sentence for the model).
    async fn plan(&self, context: PlanContext<'_>, request: &DelegateRequest) -> Result<Vec<PlannedChild>, String>;
}

/// The key that makes creating the same child twice return the first one.
pub fn child_key(parent: &str, call_id: &str, index: usize) -> String {
    format!("delegate:{parent}:{call_id}:{index}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(agent: &str, text: &str) -> Value {
        json!({"agent": agent, "task": text})
    }

    #[test]
    fn a_valid_request_is_read_and_limits_are_enforced_with_sentences() {
        let ok = DelegateRequest::parse(&json!({"tasks": [task("researcher", "Find it.")]})).unwrap();
        assert_eq!(ok.tasks[0].agent, "researcher");
        assert!(ok.tasks[0].context.is_empty() && ok.tasks[0].expected_output.is_none());

        let many: Vec<Value> = (0..9).map(|_| task("a", "x")).collect();
        assert!(DelegateRequest::parse(&json!({"tasks": many})).unwrap_err().contains("at most 8"));
        assert!(DelegateRequest::parse(&json!({"tasks": []})).unwrap_err().contains("at least one"));
        assert!(DelegateRequest::parse(&json!({"tasks": [task("", "x")]})).unwrap_err().contains("names the agent"));
        assert!(DelegateRequest::parse(&json!({"tasks": [task("a", "  ")]})).unwrap_err().contains("is empty"));
        assert!(
            DelegateRequest::parse(&json!({"tasks": [{"agent": "a", "task": "x", "bogus": 1}]})).is_ok(),
            "unknown keys are ignored"
        );
        assert!(DelegateRequest::parse(&json!({"nope": 1})).unwrap_err().contains("not valid"));
        let long = "x".repeat(MAX_TASK_CHARS + 1);
        assert!(DelegateRequest::parse(&json!({"tasks": [task("a", &long)]})).unwrap_err().contains("too long"));
        let notes: Vec<String> = (0..17).map(|i| i.to_string()).collect();
        assert!(DelegateRequest::parse(&json!({"tasks": [{"agent": "a", "task": "x", "context": notes}]})).is_err());
    }

    #[test]
    fn the_schema_asks_for_tasks_and_names_the_limits() {
        let schema = DelegateRequest::schema();
        assert_eq!(schema["required"], json!(["tasks"]));
        assert_eq!(schema["properties"]["tasks"]["maxItems"], MAX_TASKS);
        assert_eq!(schema["properties"]["tasks"]["items"]["required"], json!(["agent", "task"]));
    }

    #[test]
    fn child_keys_are_stable_and_distinct() {
        assert_eq!(child_key("p", "call_1_0", 2), "delegate:p:call_1_0:2");
        assert_ne!(child_key("p", "call_1_0", 0), child_key("p", "call_1_0", 1));
    }
}
