//! `code.run` and `code.run_in_workspace` (ADR-0022).
//!
//! A person grants the language (`execute` on `code:javascript` or `code:python`). `code.run` touches nothing outside the
//! sandbox, so it does not ask. `code.run_in_workspace` also shows the program the workspace folder, read-only, so it asks every
//! time and needs a full read grant on the workspace (the per-path patterns of the file tools cannot be enforced inside a program).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use zorix_core::{Action, Permission, Tool, ToolAnnotations, ToolContext, ToolError, ToolSource, ToolSpec};

use crate::runner::{Language, Limits, Outcome, Run, Sandbox, SandboxError};

/// Longest program or input accepted.
pub const MAX_INPUT_BYTES: usize = 100_000;
/// The most text of each stream given to the model.
const SHOWN_CHARS: usize = 20_000;

fn failed(message: impl Into<String>) -> ToolError {
    ToolError::Failed(message.into())
}

/// A sandbox tool.
pub struct CodeTool {
    sandbox: Arc<Sandbox>,
    workspace: bool,
    spec: ToolSpec,
}

/// The sandbox tools.
pub fn sandbox_tools(sandbox: Arc<Sandbox>) -> Vec<Arc<dyn Tool>> {
    let schema = |with_workspace: bool| {
        json!({"type": "object", "properties": {
            "language": {"type": "string", "enum": ["javascript", "python"]},
            "code": {"type": "string", "minLength": 1, "maxLength": MAX_INPUT_BYTES, "description": if with_workspace { "The program. Files of the workspace are under /workspace (read-only); /work is a scratch folder that is deleted afterwards." } else { "The program. /work is a scratch folder that is deleted afterwards." }},
            "stdin": {"type": "string", "maxLength": MAX_INPUT_BYTES, "description": "Text for the program's standard input."}
        }, "required": ["language", "code"], "additionalProperties": false})
    };
    let describe = |with_workspace: bool| {
        format!(
            "Run a short JavaScript (QuickJS) or Python 3.12 program in an isolated sandbox and return what it printed. It has no network, no environment variables and no access to the computer's files{}. It stops after 10 seconds, 256 MB of memory or 1 MB of output. Use it to calculate, transform data or test an idea; print the result.",
            if with_workspace { " except the workspace folder, which it can read but not change" } else { "" }
        )
    };
    let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
    for workspace in [false, true] {
        let mut spec = ToolSpec::new(
            if workspace { "code.run_in_workspace" } else { "code.run" },
            if workspace { "Run code with the workspace" } else { "Run code" },
            describe(workspace),
            schema(workspace),
        );
        spec.source = ToolSource::Sandbox;
        spec.timeout_ms = 60_000;
        spec.max_output_bytes = 128 * 1024;
        spec.requires_approval = workspace;
        // A program can print anything, but it can change nothing outside its own scratch folder.
        spec.annotations = ToolAnnotations { read_only: true, destructive: false, idempotent: true, open_world: false };
        tools.push(Arc::new(CodeTool { sandbox: sandbox.clone(), workspace, spec }));
    }
    tools
}

fn shown(text: &str) -> String {
    if text.chars().count() <= SHOWN_CHARS {
        return text.to_string();
    }
    let kept: String = text.chars().take(SHOWN_CHARS).collect();
    format!("{kept}… [cut: {} more characters]", text.chars().count() - SHOWN_CHARS)
}

fn result(outcome: &Outcome, limits: &Limits) -> Value {
    let mut out = json!({
        "stdout": shown(&outcome.stdout),
        "stderr": shown(&outcome.stderr),
        "exit_code": outcome.exit_code,
        "elapsed_ms": outcome.elapsed.as_millis() as u64,
    });
    if let Some(reason) = outcome.stopped {
        out["stopped"] = json!(reason.explain(limits));
    }
    out
}

#[async_trait]
impl Tool for CodeTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let name = args.get("language").and_then(Value::as_str).unwrap_or("");
        let language = Language::parse(name).ok_or_else(|| failed("language must be javascript or python."))?;
        let needed = Permission::new(Action::Execute, format!("code:{}", language.as_str()));
        if !ctx.grants.allows(&needed) {
            return Err(failed(format!(
                "This execution has no permission to run {} programs. A person can grant it with `--allow-code {}`.",
                language.as_str(),
                language.as_str()
            )));
        }
        let code = args
            .get("code")
            .and_then(Value::as_str)
            .filter(|c| !c.is_empty())
            .ok_or_else(|| failed("code is missing."))?;
        let stdin = args.get("stdin").and_then(Value::as_str);
        if code.len() > MAX_INPUT_BYTES || stdin.is_some_and(|s| s.len() > MAX_INPUT_BYTES) {
            return Err(failed("The program or its input is larger than 100 KB."));
        }
        let workspace = if self.workspace {
            if !ctx.grants.allows(&Permission::new(Action::Read, "workspace:*")) {
                return Err(failed(
                    "Showing the workspace to a program needs permission to read all of it (`--allow-read '*'`), because what a program reads cannot be limited file by file.",
                ));
            }
            Some(
                ctx.workspace_root
                    .clone()
                    .ok_or_else(|| failed("No workspace folder is configured for this execution."))?,
            )
        } else {
            None
        };
        let limits = Limits::default();
        let outcome = self
            .sandbox
            .run(Run { language, code: code.to_string(), stdin: stdin.map(str::to_string), workspace }, limits, &ctx.cancel)
            .await
            .map_err(|e| match e {
                SandboxError::PythonMissing => failed("Python is not installed on this computer. A person can install it once with `zorix sandbox install python`."),
                other => failed(other.to_string()),
            })?;
        Ok(result(&outcome, &limits))
    }

    fn describe_call(&self, args: &Value) -> Option<String> {
        let language = args.get("language").and_then(Value::as_str)?;
        let first = args.get("code").and_then(Value::as_str).unwrap_or("").lines().next().unwrap_or("");
        Some(format!(
            "Run a {language} program{}: {}",
            if self.workspace { " that can read the workspace" } else { "" },
            if first.chars().count() > 100 {
                format!("{}…", first.chars().take(100).collect::<String>())
            } else {
                first.to_string()
            }
        ))
    }
}
