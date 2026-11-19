//! Tools: what the model can ask the runtime to do.
//!
//! The shape follows the Model Context Protocol's tool definition (ADR-0008), plus the fields ZORIX needs to govern it.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::error::Result;
use crate::ids::{ConversationId, ExecutionId, TenantId, ToolCallId};
use crate::permission::{Grants, Permission};

/// Hints about what a tool does. Like MCP's, they describe; they never grant anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolAnnotations {
    /// The tool does not change anything.
    pub read_only: bool,
    /// The tool may remove or overwrite things.
    pub destructive: bool,
    /// Calling it again with the same arguments has no further effect, so it is safe to repeat after a crash.
    pub idempotent: bool,
    /// The tool reaches outside this computer.
    pub open_world: bool,
}

impl Default for ToolAnnotations {
    /// The cautious reading, as in MCP: assume the worst until the author says otherwise.
    fn default() -> Self {
        Self { read_only: false, destructive: true, idempotent: false, open_world: true }
    }
}

impl ToolAnnotations {
    /// A tool that only reads, stays on this computer and can be repeated.
    pub fn safe_read() -> Self {
        Self { read_only: true, destructive: false, idempotent: true, open_world: false }
    }
}

/// Where a tool comes from. It is shown in events and the interface, so a person can tell a built-in tool from one a
/// third-party server supplied.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolSource {
    /// Shipped with ZORIX.
    #[default]
    Builtin,
    /// Memory: remember, search, history.
    Memory,
    /// A tool of an MCP server.
    Mcp {
        /// ZORIX's name for the server.
        server: String,
    },
    /// A native integration (HTTP, Git hosts, tickets, chat, databases).
    Integration {
        /// The connection it uses, when it has one.
        connection: Option<String>,
    },
    /// Code run in the sandbox.
    Sandbox,
}

impl ToolSource {
    /// A short label for lists and events: `builtin`, `memory`, `mcp:github`, `integration:prod-db`, `sandbox`.
    pub fn label(&self) -> String {
        match self {
            ToolSource::Builtin => "builtin".into(),
            ToolSource::Memory => "memory".into(),
            ToolSource::Mcp { server } => format!("mcp:{server}"),
            ToolSource::Integration { connection: Some(name) } => format!("integration:{name}"),
            ToolSource::Integration { connection: None } => "integration".into(),
            ToolSource::Sandbox => "sandbox".into(),
        }
    }

    /// True for tools whose description and schemas were written by someone outside ZORIX.
    pub fn is_third_party(&self) -> bool {
        matches!(self, ToolSource::Mcp { .. })
    }
}

/// The full description of a tool.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    /// Stable identifier, such as `math.evaluate`. This is the name the model uses.
    pub id: String,
    /// A name for people.
    pub name: String,
    /// What it does, written for the model.
    pub description: String,
    /// JSON Schema of the arguments.
    pub input_schema: Value,
    /// JSON Schema of the result, when known.
    #[serde(default)]
    pub output_schema: Option<Value>,
    /// Hints about its effects.
    #[serde(default)]
    pub annotations: ToolAnnotations,
    /// What an execution must be granted for this tool to run (besides `use_tool` on its id).
    #[serde(default)]
    pub required_permissions: Vec<Permission>,
    /// The author asks for a person's approval on every call.
    #[serde(default)]
    pub requires_approval: bool,
    /// The approval must come from a person looking at the call. Automatic modes (`--approve yes`) cannot give it. Used
    /// for tools that change what ZORIX remembers.
    #[serde(default)]
    pub approval_by_person_only: bool,
    /// Where the tool comes from.
    #[serde(default)]
    pub source: ToolSource,
    /// How long one call may run, in milliseconds.
    pub timeout_ms: u64,
    /// The most bytes of result kept; more is cut and marked.
    pub max_output_bytes: usize,
}

impl ToolSpec {
    /// A spec with safe defaults: a 10 second timeout and 64 KiB of output.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: description.into(),
            input_schema,
            output_schema: None,
            annotations: ToolAnnotations::default(),
            required_permissions: Vec::new(),
            requires_approval: false,
            approval_by_person_only: false,
            source: ToolSource::Builtin,
            timeout_ms: 10_000,
            max_output_bytes: 64 * 1024,
        }
    }

    /// What the model is shown about the tool.
    pub fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            id: self.id.clone(),
            description: self.description.clone(),
            input_schema: self.input_schema.clone(),
        }
    }

    /// Everything a grant set must cover for this tool to run: `use_tool` on the id, and the declared permissions.
    /// The hash that pins this definition: SHA-256 of the id, description and both schemas in canonical JSON. What the model
    /// reads is what is hashed, so a changed description is a changed tool (ADR-0018).
    pub fn definition_hash(&self) -> String {
        let value = serde_json::json!({
            "id": self.id,
            "description": self.description,
            "input_schema": self.input_schema,
            "output_schema": self.output_schema,
        });
        crate::approval::canonical_hash(&value)
    }

    /// The tool as an execution pins it.
    pub fn pin(&self) -> PinnedTool {
        PinnedTool { spec: self.clone(), hash: self.definition_hash() }
    }

    pub fn all_required_permissions(&self) -> Vec<Permission> {
        let mut all = vec![Permission::use_tool(&self.id)];
        all.extend(self.required_permissions.iter().cloned());
        all
    }
}

/// A tool as an execution saw it when it started. The run keeps offering this definition, and refuses to run a tool whose
/// definition has changed since (ADR-0020).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PinnedTool {
    /// The definition.
    pub spec: ToolSpec,
    /// Its [`ToolSpec::definition_hash`].
    pub hash: String,
}

/// The part of a tool that is sent to the model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// The tool id.
    pub id: String,
    /// What it does.
    pub description: String,
    /// JSON Schema of the arguments.
    pub input_schema: Value,
}

/// Why a tool call did not produce a result.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ToolError {
    /// A temporary problem; calling again may work.
    #[error("{0}")]
    Retryable(String),
    /// The call cannot succeed as it is (bad input, missing file, refused).
    #[error("{0}")]
    Failed(String),
}

/// What a tool is given when it runs. It has no other access to the runtime.
#[derive(Clone, Debug)]
pub struct ToolContext {
    /// The execution calling.
    pub execution_id: ExecutionId,
    /// Its tenant.
    pub tenant: TenantId,
    /// The call.
    pub call_id: ToolCallId,
    /// What the execution may do. Tools that touch resources check it again themselves.
    pub grants: Grants,
    /// Fires when the execution is cancelled or the call times out.
    pub cancel: CancellationToken,
    /// The folder file tools are confined to, when the host configured one.
    pub workspace_root: Option<PathBuf>,
    /// The conversation the execution belongs to, if any. Memory tools use it for the conversation scope.
    pub conversation: Option<ConversationId>,
    /// The agent version the execution comes from, if an agent made it. Skill and memory tools use it.
    pub agent: Option<crate::agent::AgentBinding>,
}

/// A capability the model can ask for.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Its description. The same value every time.
    fn spec(&self) -> &ToolSpec;

    /// Run it. The runtime already validated `args` against the input schema and checked permissions.
    async fn call(&self, ctx: &ToolContext, args: Value) -> std::result::Result<Value, ToolError>;

    /// A short, safe sentence about what this call would do, for approval cards ("Create issue in owner/repo"). It must
    /// not echo secrets. `None` means the card shows the arguments only.
    fn describe_call(&self, _args: &Value) -> Option<String> {
        None
    }
}

/// Where tools are registered and found.
pub trait ToolRegistry: Send + Sync {
    /// Add a tool. Fails if the id is taken or the spec is not valid.
    fn register(&self, tool: std::sync::Arc<dyn Tool>) -> Result<()>;

    /// Find a tool by id.
    fn get(&self, id: &str) -> Option<std::sync::Arc<dyn Tool>>;

    /// The specs of every registered tool, sorted by id.
    fn list(&self) -> Vec<ToolSpec>;

    /// Remove a tool. Returns whether it was there. Runs that already started keep the definition they pinned.
    fn unregister(&self, id: &str) -> bool;

    /// Add a tool, or replace the one with the same id (an MCP server that changed its tools). The spec must be valid.
    fn replace(&self, tool: std::sync::Arc<dyn Tool>) -> Result<()>;

    /// A number that changes whenever the set of tools or a definition changes, for caches and change notifications.
    fn generation(&self) -> u64;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn annotations_assume_the_worst_by_default() {
        let annotations = ToolAnnotations::default();
        assert!(annotations.destructive && annotations.open_world);
        assert!(!annotations.read_only && !annotations.idempotent);
    }

    #[test]
    fn a_tool_always_needs_permission_to_be_used() {
        let mut spec = ToolSpec::new("files.write", "Write file", "Writes a file", json!({"type": "object"}));
        spec.required_permissions.push(Permission::new(crate::Action::Write, "workspace:*"));
        let required = spec.all_required_permissions();
        assert_eq!(required[0], Permission::use_tool("files.write"));
        assert_eq!(required.len(), 2);
    }

    #[test]
    fn the_model_sees_only_the_definition() {
        let spec = ToolSpec::new("a.b", "AB", "does ab", json!({"type": "object"}));
        let definition = spec.definition();
        assert_eq!(definition.id, "a.b");
        assert_eq!(definition.description, "does ab");
    }
}
