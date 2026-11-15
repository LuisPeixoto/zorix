//! The tool registry and the executor that runs a call with validation, a timeout and an output cap.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zorix_core::{Error, Result, Tool, ToolContext, ToolError, ToolRegistry, ToolSpec};

/// Keeps tools in memory, by id.
#[derive(Default)]
pub struct InMemoryToolRegistry {
    tools: RwLock<BTreeMap<String, Arc<dyn Tool>>>,
}

impl InMemoryToolRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }
}

fn check_spec(spec: &ToolSpec) -> Result<()> {
    let ok_id = !spec.id.is_empty()
        && spec.id.len() <= 64
        && spec.id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !ok_id {
        return Err(Error::InvalidInput(format!(
            "“{}” is not a valid tool id: use 1 to 64 letters, digits, '.', '_' or '-'.",
            spec.id
        )));
    }
    if spec.timeout_ms == 0 || spec.max_output_bytes == 0 {
        return Err(Error::InvalidInput(format!("Tool {} needs a timeout and an output limit above zero.", spec.id)));
    }
    jsonschema::validator_for(&spec.input_schema)
        .map_err(|e| Error::InvalidInput(format!("Tool {} has an invalid input schema: {e}", spec.id)))?;
    if let Some(output) = &spec.output_schema {
        jsonschema::validator_for(output)
            .map_err(|e| Error::InvalidInput(format!("Tool {} has an invalid output schema: {e}", spec.id)))?;
    }
    Ok(())
}

impl ToolRegistry for InMemoryToolRegistry {
    fn register(&self, tool: Arc<dyn Tool>) -> Result<()> {
        check_spec(tool.spec())?;
        let mut tools = self.tools.write();
        let id = tool.spec().id.clone();
        if tools.contains_key(&id) {
            return Err(Error::InvalidInput(format!("A tool with the id {id} is already registered.")));
        }
        tools.insert(id, tool);
        Ok(())
    }

    fn get(&self, id: &str) -> Option<Arc<dyn Tool>> {
        self.tools.read().get(id).cloned()
    }

    fn list(&self) -> Vec<ToolSpec> {
        self.tools.read().values().map(|tool| tool.spec().clone()).collect()
    }
}

/// Why a call did not produce a usable result.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CallError {
    /// The arguments do not match the tool's input schema.
    #[error("The arguments for {tool} are not valid: {details}")]
    InvalidArguments {
        /// The tool.
        tool: String,
        /// What is wrong.
        details: String,
    },
    /// The tool ran too long.
    #[error("{tool} did not finish within {timeout_ms} ms.")]
    TimedOut {
        /// The tool.
        tool: String,
        /// The limit.
        timeout_ms: u64,
    },
    /// The execution was cancelled while the tool ran.
    #[error("The call to {0} was cancelled.")]
    Cancelled(String),
    /// The tool returned something that breaks its output schema.
    #[error("{tool} returned a result that does not match its output schema: {details}")]
    InvalidOutput {
        /// The tool.
        tool: String,
        /// What is wrong.
        details: String,
    },
    /// The tool reported an error.
    #[error("{message}")]
    Tool {
        /// What the tool said.
        message: String,
        /// Whether trying again may help.
        retryable: bool,
    },
}

impl CallError {
    /// True when the same call may be tried again.
    pub fn is_retryable(&self) -> bool {
        matches!(self, CallError::Tool { retryable: true, .. } | CallError::TimedOut { .. })
    }
}

/// Validates arguments, runs a tool within its limits and cuts the output.
#[derive(Clone, Copy, Debug, Default)]
pub struct ToolExecutor;

impl ToolExecutor {
    /// Check `args` against the tool's input schema without running anything.
    pub fn validate_arguments(spec: &ToolSpec, args: &Value) -> std::result::Result<(), CallError> {
        let validator = jsonschema::validator_for(&spec.input_schema).map_err(|e| CallError::InvalidArguments {
            tool: spec.id.clone(),
            details: format!("the tool's schema is broken: {e}"),
        })?;
        let problems: Vec<String> = validator.iter_errors(args).map(|e| e.to_string()).take(5).collect();
        if problems.is_empty() {
            Ok(())
        } else {
            Err(CallError::InvalidArguments { tool: spec.id.clone(), details: problems.join("; ") })
        }
    }

    /// Run the call. The caller has already validated the arguments and checked the policy.
    pub async fn run(tool: &dyn Tool, ctx: &ToolContext, args: Value) -> std::result::Result<Value, CallError> {
        let spec = tool.spec();
        let timeout = Duration::from_millis(spec.timeout_ms);
        // A child token: timing out stops the tool without cancelling the whole execution.
        let call_ctx = ToolContext { cancel: ctx.cancel.child_token(), ..ctx.clone() };
        let guard: CancellationToken = call_ctx.cancel.clone();
        let outcome = tokio::select! {
            result = tool.call(&call_ctx, args) => Some(result),
            _ = tokio::time::sleep(timeout) => None,
            _ = ctx.cancel.cancelled() => return Err(CallError::Cancelled(spec.id.clone())),
        };
        let Some(result) = outcome else {
            guard.cancel();
            return Err(CallError::TimedOut { tool: spec.id.clone(), timeout_ms: spec.timeout_ms });
        };
        let value = result.map_err(|error| match error {
            ToolError::Retryable(message) => CallError::Tool { message, retryable: true },
            ToolError::Failed(message) => CallError::Tool { message, retryable: false },
        })?;
        if let Some(schema) = &spec.output_schema {
            let validator = jsonschema::validator_for(schema)
                .map_err(|e| CallError::InvalidOutput { tool: spec.id.clone(), details: e.to_string() })?;
            let problems: Vec<String> = validator.iter_errors(&value).map(|e| e.to_string()).take(3).collect();
            if !problems.is_empty() {
                return Err(CallError::InvalidOutput { tool: spec.id.clone(), details: problems.join("; ") });
            }
        }
        Ok(cap_output(value, spec.max_output_bytes))
    }
}

/// Keep a result within `max_bytes` *before* it is stored or shown, so it is never cut in the middle of JSON.
/// A result that is too big becomes a small object that says so and carries the start of the text.
pub fn cap_output(value: Value, max_bytes: usize) -> Value {
    let text = value.to_string();
    if text.len() <= max_bytes {
        return value;
    }
    let budget = max_bytes.saturating_sub(120).max(16);
    let mut end = budget.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    json!({ "truncated": true, "original_bytes": text.len(), "preview": &text[..end] })
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use serde_json::json;
    use zorix_core::{ExecutionId, Grants, TenantId, ToolCallId};

    use super::*;

    struct Echo {
        spec: ToolSpec,
        delay: Duration,
        output: Value,
        fail: Option<ToolError>,
    }

    impl Echo {
        fn new(id: &str) -> Self {
            let schema = json!({"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"], "additionalProperties": false});
            let mut spec = ToolSpec::new(id, "Echo", "Echoes", schema);
            spec.timeout_ms = 200;
            Self { spec, delay: Duration::ZERO, output: json!({"ok": true}), fail: None }
        }
    }

    #[async_trait]
    impl Tool for Echo {
        fn spec(&self) -> &ToolSpec {
            &self.spec
        }

        async fn call(&self, ctx: &ToolContext, args: Value) -> std::result::Result<Value, ToolError> {
            tokio::select! {
                _ = tokio::time::sleep(self.delay) => {}
                _ = ctx.cancel.cancelled() => return Err(ToolError::Failed("stopped".into())),
            }
            match &self.fail {
                Some(error) => Err(error.clone()),
                None if self.output == json!({"ok": true}) => Ok(json!({ "echo": args["text"] })),
                None => Ok(self.output.clone()),
            }
        }
    }

    fn context() -> ToolContext {
        ToolContext {
            execution_id: ExecutionId::new(),
            tenant: TenantId::local(),
            call_id: ToolCallId("call_1_0".into()),
            grants: Grants::none(),
            cancel: CancellationToken::new(),
            workspace_root: None,
        }
    }

    #[test]
    fn the_registry_lists_tools_sorted_and_refuses_duplicates() {
        let registry = InMemoryToolRegistry::new();
        registry.register(Arc::new(Echo::new("b.tool"))).unwrap();
        registry.register(Arc::new(Echo::new("a.tool"))).unwrap();
        assert_eq!(registry.list().iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["a.tool", "b.tool"]);
        assert!(registry.get("a.tool").is_some());
        assert!(registry.get("missing").is_none());
        let error = registry.register(Arc::new(Echo::new("a.tool"))).unwrap_err();
        assert!(error.to_string().contains("already registered"));
    }

    #[test]
    fn the_registry_refuses_bad_specs() {
        let registry = InMemoryToolRegistry::new();
        assert!(registry.register(Arc::new(Echo::new("bad id"))).is_err());
        let mut broken = Echo::new("x.y");
        broken.spec.input_schema = json!({"type": 12});
        assert!(registry.register(Arc::new(broken)).is_err());
        let mut no_timeout = Echo::new("x.z");
        no_timeout.spec.timeout_ms = 0;
        assert!(registry.register(Arc::new(no_timeout)).is_err());
    }

    #[test]
    fn arguments_are_validated_against_the_schema() {
        let spec = Echo::new("t").spec;
        assert!(ToolExecutor::validate_arguments(&spec, &json!({"text": "hi"})).is_ok());
        for bad in [json!({}), json!({"text": 5}), json!({"text": "x", "extra": 1}), json!("nope")] {
            let error = ToolExecutor::validate_arguments(&spec, &bad).unwrap_err();
            assert!(matches!(error, CallError::InvalidArguments { .. }), "{bad}");
        }
    }

    #[tokio::test]
    async fn a_call_returns_the_tool_result() {
        let tool = Echo::new("t");
        let value = ToolExecutor::run(&tool, &context(), json!({"text": "hi"})).await.unwrap();
        assert_eq!(value, json!({"echo": "hi"}));
    }

    #[tokio::test]
    async fn a_slow_tool_times_out_and_is_told_to_stop() {
        let mut tool = Echo::new("t");
        tool.delay = Duration::from_secs(30);
        let error = ToolExecutor::run(&tool, &context(), json!({"text": "hi"})).await.unwrap_err();
        assert_eq!(error, CallError::TimedOut { tool: "t".into(), timeout_ms: 200 });
        assert!(error.is_retryable());
    }

    #[tokio::test]
    async fn cancelling_the_execution_stops_the_call() {
        let mut tool = Echo::new("t");
        tool.delay = Duration::from_secs(30);
        tool.spec.timeout_ms = 60_000;
        let ctx = context();
        let cancel = ctx.cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            cancel.cancel();
        });
        let error = ToolExecutor::run(&tool, &ctx, json!({"text": "hi"})).await.unwrap_err();
        assert_eq!(error, CallError::Cancelled("t".into()));
    }

    #[tokio::test]
    async fn tool_errors_keep_their_retryable_flag() {
        let mut tool = Echo::new("t");
        tool.fail = Some(ToolError::Retryable("busy".into()));
        let retry = ToolExecutor::run(&tool, &context(), json!({"text": "x"})).await.unwrap_err();
        assert!(retry.is_retryable());
        tool.fail = Some(ToolError::Failed("no such file".into()));
        let fatal = ToolExecutor::run(&tool, &context(), json!({"text": "x"})).await.unwrap_err();
        assert!(!fatal.is_retryable());
        assert_eq!(fatal.to_string(), "no such file");
    }

    #[tokio::test]
    async fn output_is_checked_against_the_output_schema() {
        let mut tool = Echo::new("t");
        tool.spec.output_schema = Some(json!({"type": "object", "required": ["echo"]}));
        assert!(ToolExecutor::run(&tool, &context(), json!({"text": "x"})).await.is_ok());
        tool.output = json!({"other": 1});
        let error = ToolExecutor::run(&tool, &context(), json!({"text": "x"})).await.unwrap_err();
        assert!(matches!(error, CallError::InvalidOutput { .. }));
    }

    #[test]
    fn big_output_is_replaced_by_a_marked_preview_that_is_still_valid_json() {
        let small = json!({"a": 1});
        assert_eq!(cap_output(small.clone(), 100), small);
        let big = json!({"text": "é".repeat(5_000)});
        let capped = cap_output(big, 500);
        assert_eq!(capped["truncated"], json!(true));
        assert!(capped.to_string().len() <= 500, "{}", capped.to_string().len());
        assert!(capped["original_bytes"].as_u64().unwrap() > 5_000);
    }
}
