//! Turning events and records into text for people.

use serde_json::Value;
use zorix_core::{EventKind, Execution, ExecutionEvent, Timestamp};

/// Cut a string to `max` characters, with an ellipsis.
pub fn shorten(text: &str, max: usize) -> String {
    let flat: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    if flat.chars().count() <= max {
        flat
    } else {
        format!("{}…", flat.chars().take(max.saturating_sub(1)).collect::<String>())
    }
}

fn clock(at: Timestamp) -> String {
    // 2026-11-15T20:07:25.289Z -> 20:07:25.289
    let text = at.to_rfc3339();
    text.split('T').nth(1).map(|t| t.trim_end_matches('Z').to_string()).unwrap_or(text)
}

fn text(payload: &Value, key: &str) -> String {
    match &payload[key] {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn count(payload: &Value, key: &str) -> u64 {
    payload[key].as_u64().unwrap_or(0)
}

/// One line describing an event.
pub fn event_line(event: &ExecutionEvent) -> String {
    let p = &event.payload;
    let detail = match event.kind {
        EventKind::Created => format!("model {}", text(p, "model")),
        EventKind::ModelRequested => {
            format!("step {}: {} messages, {} tools", count(p, "iteration"), count(p, "messages"), count(p, "tools"))
        }
        EventKind::ModelResponded => {
            let tokens = count(&p["usage"], "input_tokens") + count(&p["usage"], "output_tokens");
            format!("{}, {} tool call(s), {tokens} tokens", text(p, "finish"), count(p, "tool_calls"))
        }
        EventKind::ModelRetrying => {
            format!(
                "attempt {} failed ({}); trying again in {} ms",
                count(p, "attempt"),
                text(p, "error"),
                count(p, "delay_ms")
            )
        }
        EventKind::ToolRequested => format!("{} {}", text(p, "tool"), shorten(&p["arguments"].to_string(), 100)),
        EventKind::ToolStarted => format!("{} (attempt {})", text(p, "tool"), count(p, "attempt")),
        EventKind::ToolCompleted => format!("{} in {} ms", text(p, "tool"), count(p, "duration_ms")),
        EventKind::ToolFailed => format!("{}: {}", text(p, "tool"), shorten(&text(p, "error"), 120)),
        EventKind::ToolDenied => {
            format!("{}: {} (by {})", text(p, "tool"), shorten(&text(p, "reason"), 120), text(p, "by"))
        }
        EventKind::WaitingApproval => format!("{}: {}", text(p, "tool"), text(p, "reason")),
        EventKind::ApprovalGranted | EventKind::ApprovalDenied => {
            let note = text(p, "note");
            let note = if note.is_empty() { String::new() } else { format!(" — {note}") };
            format!("{} by {}{note}", text(p, "tool"), text(p, "decided_by"))
        }
        EventKind::ResultRejected => text(p, "reason"),
        EventKind::Completed => {
            format!("{} characters in {} step(s)", count(p, "result_chars"), count(p, "iterations"))
        }
        EventKind::Failed => format!("{}: {}", text(p, "kind"), text(p, "message")),
        EventKind::Cancelled => text(p, "reason"),
        EventKind::Progress
        | EventKind::Checkpoint
        | EventKind::Queued
        | EventKind::Started
        | EventKind::Paused
        | EventKind::Resumed
        | EventKind::Recovered => String::new(),
    };
    let name = event.kind.name().trim_start_matches("execution.");
    format!("{}  #{:<4} {:<22} {}", clock(event.at), event.seq, name, detail).trim_end().to_string()
}

/// The line printed under a finished execution.
pub fn footer(execution: &Execution) -> String {
    let usage = &execution.usage;
    format!(
        "execution {} · {} step(s) · {} tool call(s) · {} tokens",
        execution.id,
        execution.iteration,
        usage.tool_calls,
        usage.total_tokens()
    )
}

/// A short table row for `list`.
pub fn execution_row(execution: &Execution) -> String {
    format!(
        "{}  {:<20} {}  {:<8} {}",
        execution.id,
        execution.state,
        execution.created_at.to_rfc3339().chars().take(19).collect::<String>().replace('T', " "),
        execution.request.model,
        shorten(&execution.request.prompt, 48)
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use zorix_core::{EVENT_SCHEMA_VERSION, ExecutionId};

    use super::*;

    fn line(kind: EventKind, payload: Value) -> String {
        event_line(&ExecutionEvent {
            seq: 7,
            execution_id: ExecutionId::new(),
            at: Timestamp(1_700_000_000_123),
            schema_version: EVENT_SCHEMA_VERSION,
            kind,
            payload,
        })
    }

    #[test]
    fn each_kind_reads_well() {
        let cases = [
            (EventKind::Created, json!({"model": "demo"}), "model demo"),
            (EventKind::Queued, json!({}), "queued"),
            (
                EventKind::ModelRequested,
                json!({"iteration": 1, "messages": 4, "tools": 2}),
                "step 1: 4 messages, 2 tools",
            ),
            (
                EventKind::ModelResponded,
                json!({"finish": "tool_calls", "tool_calls": 1, "usage": {"input_tokens": 40, "output_tokens": 2}}),
                "tool_calls, 1 tool call(s), 42 tokens",
            ),
            (
                EventKind::ToolRequested,
                json!({"tool": "math.evaluate", "arguments": {"expression": "17*23"}}),
                "math.evaluate {\"expression\":\"17*23\"}",
            ),
            (EventKind::ToolCompleted, json!({"tool": "math.evaluate", "duration_ms": 3}), "math.evaluate in 3 ms"),
            (
                EventKind::ToolDenied,
                json!({"tool": "files.write", "reason": "no permission", "by": "policy"}),
                "files.write: no permission (by policy)",
            ),
            (
                EventKind::WaitingApproval,
                json!({"tool": "notes.put", "reason": "asks every time"}),
                "notes.put: asks every time",
            ),
            (
                EventKind::ApprovalDenied,
                json!({"tool": "notes.put", "decided_by": "luis", "note": "not now"}),
                "notes.put by luis — not now",
            ),
            (
                EventKind::Failed,
                json!({"kind": "limit_exceeded", "message": "Stopped after 3 model steps"}),
                "limit_exceeded: Stopped after 3 model steps",
            ),
            (EventKind::Completed, json!({"result_chars": 12, "iterations": 2}), "12 characters in 2 step(s)"),
        ];
        for (kind, payload, wanted) in cases {
            let text = line(kind, payload);
            assert!(text.ends_with(wanted), "{kind}: {text:?} should end with {wanted:?}");
            assert!(text.starts_with("22:13:20.123  #7"), "{text}");
        }
    }

    #[test]
    fn long_and_multiline_text_is_shortened_to_one_line() {
        let long = "word ".repeat(100);
        let shortened = shorten(&format!("a\nb\t{long}"), 30);
        assert_eq!(shortened.chars().count(), 30);
        assert!(!shortened.contains('\n') && shortened.ends_with('…'));
        assert_eq!(shorten("short", 30), "short");
    }
}
