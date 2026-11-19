//! The tools a run uses to work with its agent's skills: `skill.load`, `skill.read` and `skill.run`.
//!
//! They see only the skills the run's agent pinned. A script runs only in the sandbox, with the skill mounted read-only.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use zorix_core::{
    Action, AgentBinding, AgentStore, Permission, SkillPin, SkillRecord, Tool, ToolAnnotations, ToolContext, ToolError,
    ToolSource, ToolSpec,
};
use zorix_sandbox::{Language, Limits, Run, Sandbox, SandboxError};

use super::frontmatter::parse_skill_md;
use super::library::{SkillError, SkillLibrary};

/// The most bytes of a file `skill.read` returns.
pub const MAX_READ: usize = 256 * 1024;

fn failed(text: impl Into<String>) -> ToolError {
    ToolError::Failed(text.into())
}

/// The three skill tools.
pub fn skill_tools(
    store: Arc<dyn AgentStore>,
    library: Arc<SkillLibrary>,
    sandbox: Arc<Sandbox>,
) -> Vec<Arc<dyn Tool>> {
    let shared = Shared { store, library };
    let name = json!({"type": "string", "minLength": 1, "maxLength": 64, "description": "The skill's name, as listed in <available_skills>."});
    let mut load = ToolSpec::new(
        "skill.load",
        "Load a skill",
        "Load one of your skills: its instructions, and the list of files that come with it. Do this when a skill's description fits the task.",
        json!({"type": "object", "properties": {"name": name.clone()}, "required": ["name"], "additionalProperties": false}),
    );
    load.annotations = ToolAnnotations::safe_read();
    load.max_output_bytes = 128 * 1024;
    let mut read = ToolSpec::new(
        "skill.read",
        "Read a file of a skill",
        "Read a text file that comes with one of your skills (a reference, a template). Use a path from the skill's file list.",
        json!({"type": "object", "properties": {
            "name": name.clone(),
            "path": {"type": "string", "minLength": 1, "maxLength": 260, "description": "A path from the skill's file list, such as references/REFERENCE.md."}
        }, "required": ["name", "path"], "additionalProperties": false}),
    );
    read.annotations = ToolAnnotations::safe_read();
    read.max_output_bytes = MAX_READ + 4 * 1024;
    let mut run = ToolSpec::new(
        "skill.run",
        "Run a script of a skill",
        "Run a Python or JavaScript script that comes with one of your skills, in the code sandbox: no network, the skill's own files read-only, the standard library only. Give it arguments and standard input if the skill says to.",
        json!({"type": "object", "properties": {
            "name": name,
            "script": {"type": "string", "minLength": 1, "maxLength": 260, "description": "A script from the skill's file list, such as scripts/extract.py."},
            "args": {"type": "array", "items": {"type": "string", "maxLength": 1000}, "maxItems": 20},
            "stdin": {"type": "string", "maxLength": 100000}
        }, "required": ["name", "script"], "additionalProperties": false}),
    );
    run.source = ToolSource::Sandbox;
    run.timeout_ms = 30_000;
    run.max_output_bytes = 128 * 1024;
    run.annotations = ToolAnnotations { read_only: true, destructive: false, idempotent: true, open_world: false };
    vec![
        Arc::new(Load { shared: shared.clone(), spec: load }),
        Arc::new(Read { shared: shared.clone(), spec: read }),
        Arc::new(RunScript { shared, sandbox, spec: run }),
    ]
}

#[derive(Clone)]
struct Shared {
    store: Arc<dyn AgentStore>,
    library: Arc<SkillLibrary>,
}

impl Shared {
    /// The pin and the record of a skill the run's agent has. Anything else does not exist for the run.
    async fn find(&self, ctx: &ToolContext, args: &Value) -> Result<(SkillPin, SkillRecord), ToolError> {
        let binding: &AgentBinding =
            ctx.agent.as_ref().ok_or_else(|| failed("Skills belong to an agent, and this run has none."))?;
        let name = args.get("name").and_then(Value::as_str).unwrap_or("");
        let pin = binding.skills.iter().find(|p| p.name == name).ok_or_else(|| {
            let names: Vec<&str> = binding.skills.iter().map(|p| p.name.as_str()).collect();
            failed(format!(
                "{name} is not one of your skills. Yours: {}.",
                if names.is_empty() { "none".into() } else { names.join(", ") }
            ))
        })?;
        let record = self
            .store
            .skill_version(&ctx.tenant, &pin.name, &pin.hash)
            .await
            .map_err(|e| failed(e.to_string()))?
            .ok_or_else(|| {
                failed(format!("The skill {name} is not in the library any more. A person has to import it again."))
            })?;
        Ok((pin.clone(), record))
    }

    fn listed(record: &SkillRecord, path: &str) -> Result<(), ToolError> {
        if record.files.iter().any(|f| f.path == path) {
            Ok(())
        } else {
            Err(failed(format!(
                "The skill {} has no file {path}. Its files: {}.",
                record.name,
                record.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>().join(", ")
            )))
        }
    }
}

fn skill_failure(error: SkillError) -> ToolError {
    failed(error.to_string())
}

struct Load {
    shared: Shared,
    spec: ToolSpec,
}

#[async_trait]
impl Tool for Load {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let (pin, record) = self.shared.find(ctx, &args).await?;
        let bytes = self.shared.library.read(&pin.hash, "SKILL.md", 200 * 1024).map_err(skill_failure)?;
        let text = String::from_utf8(bytes).map_err(|_| failed("SKILL.md is not text."))?;
        let (_, body) = parse_skill_md(&text, None).map_err(failed)?;
        Ok(json!({
            "name": record.name,
            "description": record.description,
            "instructions": body,
            "files": record.files.iter().filter(|f| f.path != "SKILL.md").map(|f| json!({"path": f.path, "bytes": f.size})).collect::<Vec<_>>(),
            "note": "Read other files with skill.read; run scripts with skill.run.",
        }))
    }

    fn describe_call(&self, args: &Value) -> Option<String> {
        Some(format!("Load the skill {}", args.get("name")?.as_str()?))
    }
}

struct Read {
    shared: Shared,
    spec: ToolSpec,
}

#[async_trait]
impl Tool for Read {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let (pin, record) = self.shared.find(ctx, &args).await?;
        let path = args.get("path").and_then(Value::as_str).unwrap_or("");
        Shared::listed(&record, path)?;
        let bytes = self.shared.library.read(&pin.hash, path, MAX_READ).map_err(skill_failure)?;
        match String::from_utf8(bytes) {
            Ok(text) => Ok(json!({"path": path, "text": text})),
            Err(e) => Err(failed(format!("{path} is not a text file ({} bytes).", e.as_bytes().len()))),
        }
    }

    fn describe_call(&self, args: &Value) -> Option<String> {
        Some(format!("Read {} of the skill {}", args.get("path")?.as_str()?, args.get("name")?.as_str()?))
    }
}

struct RunScript {
    shared: Shared,
    sandbox: Arc<Sandbox>,
    spec: ToolSpec,
}

/// The program that runs a skill's script from inside the sandbox: the script's own path, its arguments, and nothing else.
fn launcher(language: Language, script: &str, args: &[String]) -> String {
    let path = format!("/skill/{script}");
    match language {
        Language::Python => format!(
            "import runpy, sys\nsys.argv = {}\nrunpy.run_path({}, run_name='__main__')\n",
            serde_json::to_string(&std::iter::once(path.clone()).chain(args.iter().cloned()).collect::<Vec<_>>())
                .unwrap_or_else(|_| "[]".into()),
            serde_json::to_string(&path).unwrap_or_default(),
        ),
        Language::JavaScript => format!(
            "globalThis.scriptArgs = {};\nstd.loadScript({});\n",
            serde_json::to_string(&args).unwrap_or_else(|_| "[]".into()),
            serde_json::to_string(&path).unwrap_or_default(),
        ),
    }
}

#[async_trait]
impl Tool for RunScript {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let (pin, record) = self.shared.find(ctx, &args).await?;
        let script = args.get("script").and_then(Value::as_str).unwrap_or("");
        Shared::listed(&record, script)?;
        let language = match script.rsplit('.').next() {
            Some("py") if script.starts_with("scripts/") => Language::Python,
            Some("js") if script.starts_with("scripts/") => Language::JavaScript,
            _ => return Err(failed("Only scripts/*.py and scripts/*.js can be run.")),
        };
        let needed = Permission::new(Action::Execute, format!("code:{}", language.as_str()));
        if !ctx.grants.allows(&needed) {
            return Err(failed(format!(
                "This execution has no permission to run {} programs. A person can grant it with `--allow-code {}` (or access `code:{}` on the agent).",
                language.as_str(),
                language.as_str(),
                language.as_str()
            )));
        }
        let folder = self.shared.library.path(&pin.hash).map_err(skill_failure)?;
        let extra: Vec<String> = args
            .get("args")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let limits = Limits::default();
        let outcome = self
            .sandbox
            .run(
                Run {
                    language,
                    code: launcher(language, script, &extra),
                    stdin: args.get("stdin").and_then(Value::as_str).map(str::to_string),
                    workspace: None,
                    mounts: vec![(folder, "/skill".into())],
                },
                limits,
                &ctx.cancel,
            )
            .await
            .map_err(|e| match e {
                SandboxError::PythonMissing => failed("Python is not installed on this computer. A person can install it once with `zorix sandbox install python`."),
                other => failed(other.to_string()),
            })?;
        let mut out = zorix_sandbox::tool::result(&outcome, &limits);
        if language == Language::Python && outcome.exit_code != 0 && outcome.stderr.contains("ModuleNotFoundError") {
            out["hint"] = json!(
                "The sandbox has only Python's standard library, and no network, so a script that imports another package cannot run here."
            );
        }
        Ok(out)
    }

    fn describe_call(&self, args: &Value) -> Option<String> {
        Some(format!("Run {} of the skill {}", args.get("script")?.as_str()?, args.get("name")?.as_str()?))
    }
}
