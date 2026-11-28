//! The two smallest tools: the clock and a text counter.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use zorix_core::{Clock, SystemClock, Tool, ToolAnnotations, ToolContext, ToolError, ToolSpec};

/// `clock.now`: the current time, from a clock that tests can replace.
pub struct ClockTool {
    spec: ToolSpec,
    clock: Arc<dyn Clock>,
}

impl ClockTool {
    /// A tool that reads the computer's clock.
    pub fn new() -> Self {
        Self::with_clock(Arc::new(SystemClock))
    }

    /// A tool that reads another clock.
    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        let mut spec = ToolSpec::new(
            "clock.now",
            "Clock",
            "Returns the current date and time in UTC.",
            json!({"type": "object", "properties": {}, "additionalProperties": false}),
        );
        spec.annotations = ToolAnnotations::safe_read();
        spec.timeout_ms = 1_000;
        Self { spec, clock }
    }
}

impl Default for ClockTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ClockTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, _ctx: &ToolContext, _args: Value) -> Result<Value, ToolError> {
        let now = self.clock.now();
        Ok(json!({ "now": now.to_rfc3339(), "unix_ms": now.millis() }))
    }
}

/// `text.stats`: counts characters, words and lines.
pub struct TextStatsTool {
    spec: ToolSpec,
}

impl TextStatsTool {
    /// A new tool.
    pub fn new() -> Self {
        let schema = json!({
            "type": "object",
            "properties": {"text": {"type": "string", "maxLength": 100000}},
            "required": ["text"],
            "additionalProperties": false
        });
        let mut spec =
            ToolSpec::new("text.stats", "Text statistics", "Counts the characters, words and lines of a text.", schema);
        spec.annotations = ToolAnnotations::safe_read();
        spec.timeout_ms = 1_000;
        Self { spec }
    }
}

impl Default for TextStatsTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for TextStatsTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, _ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let text = args["text"].as_str().unwrap_or_default();
        Ok(
            json!({ "characters": text.chars().count(), "words": text.split_whitespace().count(), "lines": text.lines().count() }),
        )
    }
}

#[cfg(test)]
mod tests {
    use zorix_core::{ExecutionId, Grants, ManualClock, TenantId, Timestamp, ToolCallId};

    use super::*;

    fn context() -> ToolContext {
        ToolContext {
            execution_id: ExecutionId::new(),
            tenant: TenantId::local(),
            call_id: ToolCallId("c".into()),
            grants: Grants::none(),
            cancel: tokio_util::sync::CancellationToken::new(),
            workspace_root: None,
            conversation: None,
            agent: None,
            host: None,
        }
    }

    #[tokio::test]
    async fn the_clock_reads_the_clock_it_was_given() {
        let tool = ClockTool::with_clock(Arc::new(ManualClock::new(Timestamp(1_700_000_000_123))));
        let value = tool.call(&context(), json!({})).await.unwrap();
        assert_eq!(value, json!({"now": "2023-11-14T22:13:20.123Z", "unix_ms": 1_700_000_000_123_i64}));
    }

    #[tokio::test]
    async fn text_is_counted() {
        let value = TextStatsTool::new().call(&context(), json!({"text": "héllo wörld\nsecond line"})).await.unwrap();
        assert_eq!(value, json!({"characters": 23, "words": 4, "lines": 2}));
        let empty = TextStatsTool::new().call(&context(), json!({"text": ""})).await.unwrap();
        assert_eq!(empty, json!({"characters": 0, "words": 0, "lines": 0}));
    }
}
