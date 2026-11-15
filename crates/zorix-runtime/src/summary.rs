//! A readable summary of one execution: what ran, with which model and tools, how long each step took, what it used and
//! what came out. Built from the stored events and records, so it works on finished and running executions alike.

use serde::{Deserialize, Serialize};
use zorix_core::{
    EventKind, Execution, ExecutionDetail, ExecutionEvent, ExecutionId, ExecutionState, Failure, ToolCallStatus, Usage,
};

/// One tool call, summarised.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolSummary {
    /// The call.
    pub call_id: String,
    /// The tool.
    pub tool: String,
    /// How it ended.
    pub status: ToolCallStatus,
    /// How many times it was started.
    pub attempts: u32,
    /// How long the last run took, in milliseconds.
    pub duration_ms: Option<u64>,
}

/// How long one step took.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepTiming {
    /// `model` or `tool`.
    pub kind: String,
    /// The model or the tool.
    pub name: String,
    /// Milliseconds from asking to answering, retries included.
    pub duration_ms: u64,
}

/// What happened in an execution.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExecutionSummary {
    /// The execution.
    pub execution_id: ExecutionId,
    /// Where it stands.
    pub state: ExecutionState,
    /// The model asked for.
    pub model: String,
    /// Model-driven iterations started.
    pub iterations: u32,
    /// Model calls that answered.
    pub model_calls: u32,
    /// Retries of model calls.
    pub model_retries: u32,
    /// The tool calls, in order.
    pub tool_calls: Vec<ToolSummary>,
    /// Timing of every model and tool step, in order.
    pub steps: Vec<StepTiming>,
    /// Time from creation to the end, once ended.
    pub total_ms: Option<u64>,
    /// Time spent working (waiting for a person excluded).
    pub active_ms: u64,
    /// What it consumed.
    pub usage: Usage,
    /// The first 200 characters of the answer.
    pub result_preview: Option<String>,
    /// Why it failed.
    pub failure: Option<Failure>,
    /// How many events it produced.
    pub events: usize,
}

/// Summarise an execution from its detail and its events.
pub fn summarize(detail: &ExecutionDetail, events: &[ExecutionEvent]) -> ExecutionSummary {
    let execution: &Execution = &detail.execution;
    let mut steps = Vec::new();
    let mut model_started = None;
    let mut model_retries = 0;
    for event in events {
        match event.kind {
            EventKind::ModelRequested => model_started = Some(event.at),
            EventKind::ModelRetrying => model_retries += 1,
            EventKind::ModelResponded => {
                if let Some(started) = model_started.take() {
                    let name = event.payload["model"].as_str().unwrap_or(&execution.request.model).to_string();
                    steps.push(StepTiming { kind: "model".into(), name, duration_ms: event.at.since(started) });
                }
            }
            EventKind::ToolCompleted | EventKind::ToolFailed => {
                let name = event.payload["tool"].as_str().unwrap_or("?").to_string();
                let duration_ms = event.payload["duration_ms"].as_u64().unwrap_or(0);
                steps.push(StepTiming { kind: "tool".into(), name, duration_ms });
            }
            _ => {}
        }
    }
    let tool_calls = detail
        .tool_calls
        .iter()
        .map(|record| ToolSummary {
            call_id: record.call_id.0.clone(),
            tool: record.tool.clone(),
            status: record.status,
            attempts: record.attempts,
            duration_ms: match (record.started_at, record.finished_at) {
                (Some(started), Some(finished)) => Some(finished.since(started)),
                _ => None,
            },
        })
        .collect();
    ExecutionSummary {
        execution_id: execution.id,
        state: execution.state,
        model: execution.request.model.clone(),
        iterations: execution.iteration,
        model_calls: execution.usage.model_calls,
        model_retries,
        tool_calls,
        steps,
        total_ms: execution.finished_at.map(|end| end.since(execution.created_at)),
        active_ms: execution.active_ms,
        usage: execution.usage,
        result_preview: execution.result.as_ref().map(|r| r.text.chars().take(200).collect()),
        failure: execution.failure.clone(),
        events: events.len(),
    }
}
