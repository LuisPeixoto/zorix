//! Operational memory: a searchable record of what was done and decided in each finished execution (ADR-0013).
//!
//! The recorder watches the event stream for executions that end, and a background task writes the record. It never blocks
//! the engine and never fails an execution; when it falls behind, records are filled in later from what is stored.

use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::mpsc;
use tracing::{debug, warn};
use zorix_core::{
    ApprovalStatus, EventKind, EventPublisher, Execution, ExecutionEvent, ExecutionId, ExecutionRepository,
    ExecutionState, MemoryStore, OperationRecord, Timestamp, ToolCallRecord, ToolCallStatus, redact_text,
};

use crate::embedder::Embedder;
use crate::text::cut;

/// Characters of the prompt kept.
pub const PROMPT_CHARS: usize = 500;
/// Characters of the outcome kept.
pub const OUTCOME_CHARS: usize = 500;
/// Characters of a tool's arguments kept as a digest.
pub const DIGEST_CHARS: usize = 200;
/// Records waiting to be written before new ones are left for the backfill.
pub const QUEUE: usize = 256;

fn is_end(kind: EventKind) -> bool {
    matches!(kind, EventKind::Completed | EventKind::Failed | EventKind::Cancelled)
}

/// The record of a finished execution. Pure, so it can be tested without a database.
pub fn build_record(
    execution: &Execution,
    calls: &[ToolCallRecord],
    approvals: &[zorix_core::Approval],
) -> OperationRecord {
    let state = match execution.state {
        ExecutionState::Completed => "completed",
        ExecutionState::Failed => "failed",
        _ => "cancelled",
    };
    let outcome = match execution.state {
        ExecutionState::Completed => execution.result.as_ref().map(|r| r.text.clone()).unwrap_or_default(),
        ExecutionState::Failed => execution.failure.as_ref().map(|f| f.message.clone()).unwrap_or_default(),
        _ => execution.cancel_reason.clone().unwrap_or_else(|| "cancelled".into()),
    };
    let prompt = cut(&redact_text(&execution.request.prompt), PROMPT_CHARS);
    let outcome = cut(&redact_text(&outcome), OUTCOME_CHARS);

    // Tools: how many times each ran, how many went wrong, and what the first call looked like.
    let mut tools: Vec<(String, u32, u32, String)> = Vec::new();
    for call in calls {
        let bad = matches!(call.status, ToolCallStatus::Failed | ToolCallStatus::Denied) as u32;
        match tools.iter_mut().find(|t| t.0 == call.tool) {
            Some(entry) => {
                entry.1 += 1;
                entry.2 += bad;
            }
            None => {
                tools.push((call.tool.clone(), 1, bad, cut(&redact_text(&call.arguments.to_string()), DIGEST_CHARS)))
            }
        }
    }
    let answered: Vec<Value> = approvals
        .iter()
        .filter(|a| a.status != ApprovalStatus::Pending)
        .map(|a| {
            json!({
                "tool": a.tool,
                "decision": if a.status == ApprovalStatus::Approved { "approved" } else { "denied" },
                "by": a.decided_by,
                "note": a.note.as_ref().map(|n| cut(&redact_text(n), 200)),
            })
        })
        .collect();

    let mut text = format!("{prompt}\nOutcome ({state}): {outcome}");
    if !tools.is_empty() {
        let list: Vec<String> = tools.iter().map(|(tool, n, _, digest)| format!("{tool} x{n} {digest}")).collect();
        text.push_str(&format!("\nTools: {}", list.join("; ")));
    }
    for a in &answered {
        text.push_str(&format!(
            "\nApproval: {} {} by {}",
            a["tool"].as_str().unwrap_or(""),
            a["decision"].as_str().unwrap_or(""),
            a["by"].as_str().unwrap_or("")
        ));
        if let Some(note) = a["note"].as_str() {
            text.push_str(&format!(" ({note})"));
        }
    }
    let finished_at = execution.finished_at.unwrap_or(execution.updated_at);
    OperationRecord {
        execution_id: execution.id,
        tenant: execution.tenant.clone(),
        project: execution.workspace.as_ref().map(|p| p.to_string_lossy().into_owned()),
        conversation: execution.conversation.clone(),
        // The same rule as `visibility_of`: only an agent with memory of its own keeps its past work for itself.
        agent: execution.agent.as_ref().filter(|binding| binding.agent_memory).map(|binding| binding.agent),
        state: state.into(),
        finished_at,
        text,
        doc: json!({
            "prompt": prompt,
            "outcome": outcome,
            "state": state,
            "model": execution.request.model,
            "tools": tools.iter().map(|(tool, calls, failed, digest)| json!({"tool": tool, "calls": calls, "failed": failed, "digest": digest})).collect::<Vec<_>>(),
            "approvals": answered,
            "tokens": execution.usage.input_tokens + execution.usage.output_tokens,
            "finished_at": Timestamp(finished_at.millis()).millis(),
        }),
    }
}

/// Everything needed to write records, shared by the background task and the backfill.
#[derive(Clone)]
struct Writer {
    store: Arc<dyn MemoryStore>,
    repository: Arc<dyn ExecutionRepository>,
    embedder: Option<Arc<Embedder>>,
}

impl Writer {
    async fn record(&self, id: ExecutionId) -> zorix_core::Result<bool> {
        let Some(execution) = self.repository.load(id).await? else { return Ok(false) };
        // An evaluation is a test, not work: it must not turn up later as something that was done.
        if execution.request.metadata.contains_key(zorix_core::EVAL_LABEL) {
            return Ok(false);
        }
        if !matches!(execution.state, ExecutionState::Completed | ExecutionState::Failed | ExecutionState::Cancelled) {
            return Ok(false);
        }
        let calls = self.repository.tool_calls(id).await?;
        let approvals = self.repository.approvals(id).await?;
        let record = build_record(&execution, &calls, &approvals);
        let (tenant, text) = (record.tenant.clone(), record.text.clone());
        self.store.record_operation(record).await?;
        // Best effort: a vector, so it can be found by meaning.
        if let Some(embedder) = &self.embedder
            && let Ok(settings) = self.store.settings(&tenant).await
            && let Some(model) = settings.embedding_model
        {
            let hash = zorix_core::text_hash(text.trim());
            let _ = embedder
                .fill(
                    &*self.store,
                    &model,
                    settings.embedding_dimensions,
                    vec![(hash, text.trim().to_string())],
                    &tokio_util::sync::CancellationToken::new(),
                )
                .await;
        }
        Ok(true)
    }
}

/// Watches events and writes the record of every execution that ends.
pub struct OperationRecorder {
    queue: mpsc::Sender<ExecutionId>,
    writer: Writer,
}

impl OperationRecorder {
    /// Start the background task. Must be called inside a Tokio runtime.
    pub fn start(
        store: Arc<dyn MemoryStore>,
        repository: Arc<dyn ExecutionRepository>,
        embedder: Option<Arc<Embedder>>,
    ) -> Arc<Self> {
        Self::start_with_queue(store, repository, embedder, QUEUE)
    }

    /// The same with a chosen queue size (for tests).
    pub fn start_with_queue(
        store: Arc<dyn MemoryStore>,
        repository: Arc<dyn ExecutionRepository>,
        embedder: Option<Arc<Embedder>>,
        queue: usize,
    ) -> Arc<Self> {
        let (sender, mut receiver) = mpsc::channel::<ExecutionId>(queue.max(1));
        let writer = Writer { store, repository, embedder };
        let background = writer.clone();
        tokio::spawn(async move {
            while let Some(id) = receiver.recv().await {
                match background.record(id).await {
                    Ok(true) => debug!(%id, "recorded the operation"),
                    Ok(false) => {}
                    Err(error) => warn!(%error, %id, "could not record the operation"),
                }
            }
        });
        Arc::new(Self { queue: sender, writer })
    }

    /// Write the records of finished executions that have none (before this feature existed, or while the queue was full).
    /// Returns how many were written.
    pub async fn backfill(&self, batch: usize) -> zorix_core::Result<usize> {
        let mut written = 0;
        loop {
            let missing = self.writer.store.operations_missing(batch.max(1)).await?;
            if missing.is_empty() {
                break;
            }
            let before = written;
            for id in &missing {
                if self.writer.record(*id).await? {
                    written += 1;
                }
            }
            // Nothing new was written in a full pass: the rest cannot be (for example, the execution is gone).
            if written == before {
                break;
            }
        }
        Ok(written)
    }
}

impl EventPublisher for OperationRecorder {
    fn publish(&self, events: &[ExecutionEvent]) {
        for event in events.iter().filter(|e| is_end(e.kind)) {
            // Never wait: if the queue is full, the backfill will find this execution later.
            if self.queue.try_send(event.execution_id).is_err() {
                debug!(id = %event.execution_id, "the operations queue is full; the backfill will record it");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use zorix_core::{ExecutionResult, NewExecution, TaskRequest, TenantId, ToolCallId};

    use super::*;

    fn execution(state: ExecutionState) -> Execution {
        let new = NewExecution {
            tenant: Some(TenantId::local()),
            request: TaskRequest {
                prompt: "Migrate the billing database. Use key sk-abcdefghijklmnopqrstuvwxyz0123456789".into(),
                model: "sim".into(),
                ..TaskRequest::default()
            },
            workspace: Some("/work/one".into()),
            ..NewExecution::default()
        };
        let mut e = Execution::new(ExecutionId::new(), new, Timestamp(1_000));
        e.state = state;
        e.finished_at = Some(Timestamp(5_000));
        e
    }

    fn call(tool: &str, status: ToolCallStatus, arguments: Value) -> ToolCallRecord {
        ToolCallRecord {
            execution_id: ExecutionId::new(),
            call_id: ToolCallId("c".into()),
            tool: tool.into(),
            arguments,
            call_hash: "h".into(),
            status,
            result: None,
            error: None,
            attempts: 1,
            idempotent: true,
            requested_at: Timestamp(1),
            started_at: None,
            finished_at: None,
        }
    }

    #[test]
    fn a_record_says_what_was_asked_what_happened_and_which_tools_ran_without_secrets() {
        let mut e = execution(ExecutionState::Completed);
        e.result = Some(ExecutionResult { text: "Done: 3 tables migrated.".into(), structured: None });
        let calls = vec![
            call("sql.run", ToolCallStatus::Succeeded, json!({"query": "ALTER TABLE invoices"})),
            call("sql.run", ToolCallStatus::Failed, json!({"query": "DROP x"})),
            call("files.write", ToolCallStatus::Succeeded, json!({"path": "notes.md"})),
        ];
        let record = build_record(&e, &calls, &[]);
        assert_eq!(
            (record.state.as_str(), record.project.as_deref(), record.finished_at),
            ("completed", Some("/work/one"), Timestamp(5_000))
        );
        assert!(
            record.text.contains("Migrate the billing database")
                && record.text.contains("Outcome (completed): Done: 3 tables migrated.")
        );
        assert!(record.text.contains("sql.run x2") && record.text.contains("files.write x1"));
        assert!(
            !record.text.contains("sk-abcdef") && !record.doc.to_string().contains("sk-abcdef"),
            "secrets never get in"
        );
        let tools = record.doc["tools"].as_array().unwrap();
        assert_eq!(
            (tools[0]["tool"].as_str(), tools[0]["calls"].as_u64(), tools[0]["failed"].as_u64()),
            (Some("sql.run"), Some(2), Some(1))
        );
    }

    #[test]
    fn failures_and_cancellations_keep_their_reason_and_approvals_keep_who_decided() {
        let mut failed = execution(ExecutionState::Failed);
        failed.failure = Some(zorix_core::Failure::new(zorix_core::FailureKind::Internal, "the provider refused"));
        let record = build_record(&failed, &[], &[]);
        assert_eq!(record.state, "failed");
        assert!(record.text.contains("Outcome (failed): the provider refused"));
        let mut cancelled = execution(ExecutionState::Cancelled);
        cancelled.cancel_reason = Some("Cancelled from the desktop app".into());
        assert!(build_record(&cancelled, &[], &[]).text.contains("Cancelled from the desktop app"));
        let approval = zorix_core::Approval {
            id: zorix_core::ApprovalId::new(),
            execution_id: failed.id,
            call_id: ToolCallId("c".into()),
            tool: "files.write".into(),
            arguments: json!({}),
            call_hash: "h".into(),
            reason: "r".into(),
            status: ApprovalStatus::Denied,
            requested_at: Timestamp(1),
            decided_at: Some(Timestamp(2)),
            decided_by: Some("luis".into()),
            note: Some("not now".into()),
        };
        let record = build_record(&failed, &[], &[approval]);
        assert!(record.text.contains("Approval: files.write denied by luis (not now)"), "{}", record.text);
        assert_eq!(record.doc["approvals"][0]["by"], "luis");
    }

    #[test]
    fn long_text_is_cut() {
        let mut e = execution(ExecutionState::Completed);
        e.request.prompt = "word ".repeat(1_000);
        e.result = Some(ExecutionResult { text: "x".repeat(5_000), structured: None });
        let record = build_record(&e, &[], &[]);
        assert!(record.doc["prompt"].as_str().unwrap().chars().count() <= PROMPT_CHARS);
        assert!(record.doc["outcome"].as_str().unwrap().chars().count() <= OUTCOME_CHARS);
    }
}
