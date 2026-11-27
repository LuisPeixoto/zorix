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
        EventKind::ModelFallback => format!("{} → {} ({})", text(p, "from"), text(p, "to"), text(p, "reason")),
        EventKind::ContextSummarised => format!(
            "{} earlier message(s) summarised in {} characters ({} µ$)",
            count(p, "folded_messages"),
            count(p, "summary_chars"),
            count(p, "cost_micros")
        ),
        EventKind::ToolRequested => format!("{} {}", text(p, "tool"), shorten(&p["arguments"].to_string(), 100)),
        EventKind::ToolStarted => format!("{} (attempt {})", text(p, "tool"), count(p, "attempt")),
        EventKind::ToolCompleted => format!("{} in {} ms", text(p, "tool"), count(p, "duration_ms")),
        EventKind::ToolFailed => format!("{}: {}", text(p, "tool"), shorten(&text(p, "error"), 120)),
        EventKind::ToolDenied => {
            format!("{}: {} (by {})", text(p, "tool"), shorten(&text(p, "reason"), 120), text(p, "by"))
        }
        EventKind::ToolUnattended => {
            format!("{} ran without asking ({})", text(p, "tool"), text(p, "level"))
        }
        EventKind::ToolSuggested => format!("{} proposed, not run", text(p, "tool")),
        EventKind::WaitingApproval => format!("{}: {}", text(p, "tool"), text(p, "reason")),
        EventKind::ApprovalGranted | EventKind::ApprovalDenied => {
            let note = text(p, "note");
            let note = if note.is_empty() { String::new() } else { format!(" — {note}") };
            format!("{} by {}{note}", text(p, "tool"), text(p, "decided_by"))
        }
        EventKind::WaitingChildren => {
            format!("{} task(s) delegated; waiting for them", p["children"].as_array().map_or(0, Vec::len))
        }
        EventKind::ChildStarted => format!("{} started: {}", text(p, "agent"), shorten(&text(p, "task"), 80)),
        EventKind::ChildSettled => {
            format!("{} {} ({} µ$)", text(p, "agent"), text(p, "state"), count(p, "cost_micros"))
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
        "{}{}  {:<20} {}  {:<8} {}",
        if execution.parent.is_some() { "↳ " } else { "" },
        execution.id,
        execution.state,
        execution.created_at.to_rfc3339().chars().take(19).collect::<String>().replace('T', " "),
        execution.request.model,
        shorten(&execution.request.prompt, 48)
    )
}

/// The executions of one tree (the root first, as `ExecutionEngine::tree` returns them) drawn as a tree, one line each: state, who made
/// it, what it used and what it was asked.
pub fn tree_text(all: &[Execution]) -> String {
    fn line(e: &Execution) -> String {
        let who = e.agent.as_ref().map(|a| format!("{} v{}", a.slug, a.version)).unwrap_or_else(|| "plain run".into());
        format!(
            "{} {:<20} {:<22} {} step(s), ${:.4}  {}",
            e.id,
            e.state,
            who,
            e.usage.model_calls,
            e.usage.cost_micros as f64 / 1_000_000.0,
            shorten(&e.request.prompt, 50)
        )
    }
    fn walk(all: &[Execution], parent: &Execution, prefix: &str, out: &mut Vec<String>) {
        let kids: Vec<&Execution> = all.iter().filter(|e| e.parent == Some(parent.id)).collect();
        for (i, kid) in kids.iter().enumerate() {
            let last = i + 1 == kids.len();
            out.push(format!("{prefix}{}{}", if last { "└─ " } else { "├─ " }, line(kid)));
            walk(all, kid, &format!("{prefix}{}", if last { "   " } else { "│  " }), out);
        }
    }
    let Some(root) = all.first() else { return String::new() };
    let mut out = vec![line(root)];
    walk(all, root, "", &mut out);
    out.join("\n")
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
    fn a_tree_is_drawn_with_its_branches_and_a_sub_run_is_marked_in_lists() {
        use zorix_core::{NewExecution, TaskRequest};
        let make = |prompt: &str, parent: Option<&Execution>| {
            let new = NewExecution {
                parent: parent.map(|p| p.id),
                root: parent.map(|p| p.root.unwrap_or(p.id)),
                request: TaskRequest { prompt: prompt.into(), model: "demo".into(), ..TaskRequest::default() },
                ..NewExecution::default()
            };
            Execution::new(ExecutionId::new(), new, Timestamp(1))
        };
        let root = make("the question", None);
        let a = make("part a", Some(&root));
        let b = make("part b", Some(&root));
        let a1 = make("part a one", Some(&a));
        let text = tree_text(&[root.clone(), a.clone(), b.clone(), a1.clone()]);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 4);
        assert!(lines[0].contains("the question") && !lines[0].starts_with("├") && !lines[0].starts_with("└"));
        assert!(lines[1].starts_with("├─ ") && lines[1].contains("part a"));
        assert!(lines[2].starts_with("│  └─ ") && lines[2].contains("part a one"), "{}", lines[2]);
        assert!(lines[3].starts_with("└─ ") && lines[3].contains("part b"));
        assert!(execution_row(&a).starts_with("↳ ") && !execution_row(&root).starts_with("↳"));
        assert_eq!(tree_text(&[]), "");
    }

    #[test]
    fn each_kind_reads_well() {
        let cases = [
            (EventKind::Created, json!({"model": "demo"}), "model demo"),
            (EventKind::Queued, json!({}), "queued"),
            (
                EventKind::ModelFallback,
                json!({"from": "anthropic/claude-opus-5-5", "to": "ollama/qwen3", "reason": "rate_limited"}),
                "anthropic/claude-opus-5-5 → ollama/qwen3 (rate_limited)",
            ),
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
