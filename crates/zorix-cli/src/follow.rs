//! Following an execution on the terminal: printing its events, answering approvals, and pausing on Ctrl-C.

use std::io::{IsTerminal, Write};

use std::collections::HashMap;

use futures_util::StreamExt;
use futures_util::stream::{BoxStream, SelectAll};
use serde_json::Value;
use zorix_core::{
    ApprovalDecision, ApprovalId, EventKind, ExecutionEngine, ExecutionEvent, ExecutionId, ExecutionState, TenantId,
};
use zorix_runtime::Engine;

use crate::app::CliError;
use crate::cli::ApproveMode;
use crate::render::{event_line, shorten};

/// What the process exits with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    /// Completed.
    Done = 0,
    /// Failed.
    Failed = 1,
    /// Waiting for a person's approval.
    Waiting = 3,
    /// Cancelled.
    Cancelled = 4,
    /// Paused.
    Paused = 5,
}

/// Where output goes. Progress goes to stderr and the result to stdout, so `zorix run … | next-command` gets the answer.
#[derive(Clone, Copy, Debug)]
pub struct Printer {
    /// Print JSON lines to stdout instead of text.
    pub json: bool,
}

impl Printer {
    fn event(&self, event: &ExecutionEvent) {
        if self.json {
            if let Ok(text) = serde_json::to_string(event) {
                println!("{text}");
            }
        } else {
            eprintln!("{}", event_line(event));
        }
    }
}

/// The mode to use when none was given: ask on a terminal, otherwise leave the execution waiting.
pub fn default_mode() -> ApproveMode {
    if std::io::stdin().is_terminal() { ApproveMode::Ask } else { ApproveMode::Never }
}

/// The user name recorded with a decision.
pub fn decider() -> String {
    let user = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "unknown".into());
    format!("cli:{user}")
}

pub async fn ask(question: String) -> bool {
    tokio::task::spawn_blocking(move || {
        eprint!("{question} [y/N] ");
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) | Err(_) => {
                eprintln!("(no answer: treating it as no)");
                false
            }
            Ok(_) => matches!(line.trim().to_lowercase().as_str(), "y" | "yes" | "s" | "sim"),
        }
    })
    .await
    .unwrap_or(false)
}

fn describe_approval(payload: &Value) -> String {
    format!(
        "\n  Approval needed to run {}\n  with {}\n  why: {}\n",
        payload["tool"].as_str().unwrap_or("?"),
        shorten(&payload["arguments"].to_string(), 300),
        payload["reason"].as_str().unwrap_or("")
    )
}

/// Tell a person how to answer later.
pub fn hint_for_pending(id: ExecutionId, approval: &Value) {
    eprintln!(
        "{}\n  It is waiting. Answer with:\n    zorix approve {id} {approval}\n    zorix deny {id} {approval}",
        describe_approval(approval).trim_end(),
        approval = approval["approval_id"].as_str().unwrap_or("")
    );
}

/// A short name for a sub-run in the output: the agent it belongs to, or its id.
fn label_of(payload: &Value) -> String {
    payload["agent"].as_str().map(str::to_string).unwrap_or_else(|| "sub-run".into())
}

/// Follow `stream` until the execution finishes, pauses or has to wait for a person. Returns how it ended.
///
/// When the execution delegates, the sub-runs are followed too: their events are shown, indented under the agent's name, and an
/// approval asked anywhere in the tree is answered here (or left waiting) just like one of the root's own.
pub async fn follow(
    engine: &Engine,
    tenant: &TenantId,
    id: ExecutionId,
    stream: BoxStream<'static, ExecutionEvent>,
    mode: ApproveMode,
    printer: Printer,
) -> Result<Exit, CliError> {
    let mut pause_requested = false;
    let mut streams: SelectAll<BoxStream<'static, ExecutionEvent>> = SelectAll::new();
    streams.push(stream);
    let mut labels: HashMap<ExecutionId, String> = HashMap::new();
    loop {
        tokio::select! {
            next = streams.next() => {
                let Some(event) = next else { break };
                let at = event.execution_id;
                match labels.get(&at) {
                    Some(label) if !printer.json => eprintln!("  ↳ {label}  {}", event_line(&event)),
                    _ => printer.event(&event),
                }
                match event.kind {
                    EventKind::Paused if at == id => return finish(engine, tenant, id, printer).await,
                    EventKind::ChildStarted => {
                        // Follow the sub-run too, from its beginning.
                        let child: Option<ExecutionId> = event.payload["child"].as_str().and_then(|t| t.parse().ok());
                        if let Some(child) = child {
                            labels.insert(child, label_of(&event.payload));
                            if let Ok(sub) = engine.subscribe(tenant, child, 0).await {
                                streams.push(sub);
                            }
                        }
                    }
                    EventKind::WaitingApproval => {
                        let payload = &event.payload;
                        let approval: Option<ApprovalId> = payload["approval_id"].as_str().and_then(|t| t.parse().ok());
                        let Some(approval) = approval else { continue };
                        // A call that changes what ZORIX remembers is never answered by an automatic mode.
                        let person_only = payload["person_only"].as_bool().unwrap_or(false);
                        let approve = match mode {
                            ApproveMode::Yes if person_only => {
                                eprintln!("This call must be approved by a person, so --approve yes leaves it waiting.");
                                hint_for_pending(at, payload);
                                return finish(engine, tenant, id, printer).await;
                            }
                            ApproveMode::Never => {
                                hint_for_pending(at, payload);
                                return finish(engine, tenant, id, printer).await;
                            }
                            ApproveMode::Yes => true,
                            ApproveMode::No => false,
                            ApproveMode::Ask => {
                                if let Some(label) = labels.get(&at) {
                                    eprintln!("\n  Asked by {label}");
                                }
                                eprintln!("{}", describe_approval(payload));
                                ask("  Allow this?".to_string()).await
                            }
                        };
                        let decision = ApprovalDecision {
                            approve,
                            decided_by: if mode == ApproveMode::Ask { decider() } else { "cli:auto".into() },
                            note: None,
                            call_hash: payload["call_hash"].as_str().map(str::to_string),
                            automatic: mode != ApproveMode::Ask,
                        };
                        engine.decide(tenant, at, approval, decision).await?;
                    }
                    _ => {}
                }
            }
            _ = tokio::signal::ctrl_c() => {
                if pause_requested {
                    eprintln!("\nStopping now. Continue later with: zorix recover");
                    std::process::exit(130);
                }
                pause_requested = true;
                eprintln!("\nPausing at the next step (press Ctrl-C again to stop at once)…");
                let _ = engine.pause(tenant, id).await;
            }
        }
    }
    finish(engine, tenant, id, printer).await
}

/// Print how the execution ended and choose the exit code.
pub async fn finish(engine: &Engine, tenant: &TenantId, id: ExecutionId, printer: Printer) -> Result<Exit, CliError> {
    let execution = engine.get(tenant, id).await?;
    if printer.json {
        println!("{}", serde_json::json!({ "type": "result", "execution": execution }));
    }
    Ok(match execution.state {
        ExecutionState::Completed => {
            if !printer.json {
                if let Some(result) = &execution.result {
                    println!("{}", result.text);
                }
                eprintln!("\n{}", crate::render::footer(&execution));
            }
            Exit::Done
        }
        ExecutionState::Failed => {
            let why = execution.failure.map(|f| f.message).unwrap_or_default();
            eprintln!("\nFailed: {why}");
            Exit::Failed
        }
        ExecutionState::Cancelled => {
            eprintln!("\nCancelled{}.", execution.cancel_reason.map(|r| format!(": {r}")).unwrap_or_default());
            Exit::Cancelled
        }
        ExecutionState::Paused => {
            eprintln!("\nPaused. Continue with: zorix resume {id}");
            Exit::Paused
        }
        ExecutionState::WaitingForApproval => Exit::Waiting,
        ExecutionState::WaitingForChildren => {
            // Only reachable while something below waits for a person (or the program was stopped): say which.
            let waiting = engine.pending_in_tree(tenant, id).await.unwrap_or_default();
            if waiting.is_empty() {
                eprintln!("\nStopped while waiting for sub-runs. Continue with: zorix recover");
                Exit::Failed
            } else {
                for item in &waiting {
                    eprintln!(
                        "\n  {} asks to run {}. Answer with:\n    zorix approve {id} {}\n    zorix deny {id} {}",
                        item.agent.as_deref().unwrap_or("A sub-run"),
                        item.approval.tool,
                        item.approval.id,
                        item.approval.id
                    );
                }
                Exit::Waiting
            }
        }
        other => {
            eprintln!("\nStopped while {other}. Continue with: zorix recover");
            Exit::Failed
        }
    })
}
