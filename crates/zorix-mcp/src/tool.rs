//! A tool of an MCP server as the runtime sees it (ADR-0018, ADR-0020).
//!
//! The description reaches the model unchanged but labelled with where it came from. Every call asks first unless a person
//! marked this specific tool "do not ask" and the server reports it read-only. Results are data, never instructions.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zorix_core::{Tool, ToolAnnotations, ToolContext, ToolError, ToolSource, ToolSpec, ToolTrust};

use crate::client::{CallOutput, McpToolDef};
use crate::error::McpError;

/// Longest tool id the registry accepts.
const MAX_ID: usize = 64;
/// How long one call may run.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// The most text of one result item that is kept (the executor caps the whole result again).
const ITEM_TEXT_CAP: usize = 200_000;

/// Whoever can make a call to the server a tool belongs to. The manager implements it over whatever connection is up.
#[async_trait]
pub trait McpCaller: Send + Sync {
    /// Call `tool` at the server.
    async fn call(
        &self,
        tool: &str,
        arguments: Value,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<CallOutput, McpError>;
}

/// The id a server's tool gets: `mcp.<server>.<tool>`, shortened with a hash when it would be too long.
pub fn tool_id(server: &str, tool: &str) -> String {
    let id = format!("mcp.{server}.{tool}");
    if id.len() <= MAX_ID {
        return id;
    }
    let hash = zorix_core::canonical_hash(&json!(tool));
    let keep = MAX_ID.saturating_sub("mcp..-".len() + server.len() + 8);
    let head: String = tool.chars().take(keep).collect();
    format!("mcp.{server}.{head}-{}", &hash[..8])
}

/// The label the model reads in front of a server's own description.
pub fn label(server: &str) -> String {
    format!("[MCP server “{server}”]")
}

fn hint(annotations: &Value, name: &str) -> Option<bool> {
    annotations.get(name).and_then(Value::as_bool)
}

/// A tool of an MCP server.
pub struct McpTool {
    spec: ToolSpec,
    remote_name: String,
    caller: Arc<dyn McpCaller>,
}

impl McpTool {
    /// Build the tool for a definition a person approved.
    pub fn new(server: &str, definition: &McpToolDef, trust: ToolTrust, caller: Arc<dyn McpCaller>) -> McpTool {
        let read_only = hint(&definition.annotations, "readOnlyHint") == Some(true);
        let annotations = ToolAnnotations {
            read_only,
            // MCP's defaults assume the worst: destructive and open-world unless the server says otherwise.
            destructive: !read_only && hint(&definition.annotations, "destructiveHint") != Some(false),
            idempotent: hint(&definition.annotations, "idempotentHint") == Some(true) || read_only,
            open_world: hint(&definition.annotations, "openWorldHint") != Some(false),
        };
        let description = if definition.description.trim().is_empty() {
            format!("{} {}", label(server), definition.title.clone().unwrap_or_else(|| definition.name.clone()))
        } else {
            format!("{} {}", label(server), definition.description.trim())
        };
        let mut spec = ToolSpec::new(
            tool_id(server, &definition.name),
            definition.title.clone().unwrap_or_else(|| definition.name.clone()),
            description,
            definition.input_schema.clone(),
        );
        // The runtime checks results against an output schema; ours wrap the server's result, so the server's schema is not used.
        spec.output_schema = None;
        spec.annotations = annotations;
        spec.requires_approval = trust != ToolTrust::ReadOnlyNoAsk;
        spec.timeout_ms = CALL_TIMEOUT.as_millis() as u64 + 5_000;
        spec.source = ToolSource::Mcp { server: server.to_string() };
        McpTool { spec, remote_name: definition.name.clone(), caller }
    }
}

fn clip(text: &str) -> String {
    if text.len() <= ITEM_TEXT_CAP {
        return text.to_string();
    }
    let mut end = ITEM_TEXT_CAP;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… [cut: {} more characters]", &text[..end], text[end..].chars().count())
}

/// What the model gets of one content item. Pictures and sound are not passed on; the model is told they were there.
fn item(raw: &Value) -> Value {
    match raw.get("type").and_then(Value::as_str) {
        Some("text") => json!({"type": "text", "text": clip(raw.get("text").and_then(Value::as_str).unwrap_or(""))}),
        Some(kind @ ("image" | "audio")) => {
            let bytes = raw.get("data").and_then(Value::as_str).map(str::len).unwrap_or(0);
            json!({"type": kind, "note": format!("A {kind} ({}, about {} KB) was returned and is not shown to you.", raw.get("mimeType").and_then(Value::as_str).unwrap_or("unknown type"), bytes / 1366)})
        }
        Some("resource_link") => json!({
            "type": "resource_link",
            "uri": raw.get("uri"),
            "name": raw.get("name"),
            "description": raw.get("description"),
        }),
        Some("resource") => {
            let resource = raw.get("resource").cloned().unwrap_or(Value::Null);
            match resource.get("text").and_then(Value::as_str) {
                Some(body) => json!({"type": "resource", "uri": resource.get("uri"), "text": clip(body)}),
                None => {
                    json!({"type": "resource", "uri": resource.get("uri"), "note": "A binary resource was returned and is not shown to you."})
                }
            }
        }
        other => {
            json!({"type": other.unwrap_or("unknown"), "note": "An item of a kind ZORIX does not show was returned."})
        }
    }
}

fn text_of(items: &[Value]) -> String {
    items.iter().filter_map(|i| i.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n")
}

#[async_trait]
impl Tool for McpTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let output = self.caller.call(&self.remote_name, args, CALL_TIMEOUT, &ctx.cancel).await.map_err(|error| {
            // A call is only repeated when it is safe to repeat.
            if error.is_retryable() && self.spec.annotations.idempotent {
                ToolError::Retryable(error.to_string())
            } else {
                ToolError::Failed(error.to_string())
            }
        })?;
        let items: Vec<Value> = output.content.iter().map(item).collect();
        if output.is_error {
            let said = text_of(&items);
            return Err(ToolError::Failed(if said.is_empty() {
                "The tool reported an error without saying what it was.".into()
            } else {
                said
            }));
        }
        let mut result = json!({"content": items});
        if let Some(structured) = output.structured {
            result["structured"] = structured;
        }
        Ok(result)
    }

    fn describe_call(&self, args: &Value) -> Option<String> {
        let server = match &self.spec.source {
            ToolSource::Mcp { server } => server.as_str(),
            _ => "an MCP server",
        };
        let names: Vec<&str> =
            args.as_object().map(|o| o.keys().map(String::as_str).take(4).collect()).unwrap_or_default();
        Some(if names.is_empty() {
            format!("Run “{}” on the MCP server “{server}”", self.remote_name)
        } else {
            format!("Run “{}” on the MCP server “{server}” with {}", self.remote_name, names.join(", "))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Nobody;

    #[async_trait]
    impl McpCaller for Nobody {
        async fn call(&self, _: &str, _: Value, _: Duration, _: &CancellationToken) -> Result<CallOutput, McpError> {
            Err(McpError::Closed)
        }
    }

    fn def(name: &str, annotations: Value) -> McpToolDef {
        McpToolDef {
            name: name.into(),
            title: None,
            description: "Does a thing.".into(),
            input_schema: json!({"type": "object"}),
            output_schema: Some(json!({"type": "object"})),
            annotations,
        }
    }

    #[test]
    fn ids_are_namespaced_and_stay_within_the_limit_without_colliding() {
        assert_eq!(tool_id("github", "create_issue"), "mcp.github.create_issue");
        let long_a = tool_id("github", &format!("{}a", "x".repeat(100)));
        let long_b = tool_id("github", &format!("{}b", "x".repeat(100)));
        assert!(long_a.len() <= 64 && long_b.len() <= 64);
        assert_ne!(long_a, long_b, "two long names that share a start get different ids");
        assert!(long_a.starts_with("mcp.github.x"));
    }

    #[test]
    fn the_model_reads_the_servers_words_labelled_and_the_server_cannot_waive_the_question() {
        let tool = McpTool::new(
            "github",
            &def("create_issue", json!({"readOnlyHint": true})),
            ToolTrust::Ask,
            Arc::new(Nobody),
        );
        let spec = tool.spec();
        assert!(spec.description.starts_with("[MCP server “github”] Does a thing."));
        assert!(spec.requires_approval, "a server's hint does not turn the question off");
        assert_eq!(spec.source, ToolSource::Mcp { server: "github".into() });
        assert!(spec.output_schema.is_none());
        let no_ask = McpTool::new(
            "github",
            &def("list", json!({"readOnlyHint": true})),
            ToolTrust::ReadOnlyNoAsk,
            Arc::new(Nobody),
        );
        assert!(!no_ask.spec().requires_approval, "only a person's choice does");
    }

    #[test]
    fn missing_hints_mean_the_worst_as_the_standard_says() {
        let spec = McpTool::new("s", &def("t", json!({})), ToolTrust::Ask, Arc::new(Nobody)).spec;
        assert!(
            spec.annotations.destructive
                && spec.annotations.open_world
                && !spec.annotations.read_only
                && !spec.annotations.idempotent
        );
        let spec = McpTool::new(
            "s",
            &def("t", json!({"readOnlyHint": true, "openWorldHint": false})),
            ToolTrust::Ask,
            Arc::new(Nobody),
        )
        .spec;
        assert!(
            spec.annotations.read_only
                && !spec.annotations.destructive
                && !spec.annotations.open_world
                && spec.annotations.idempotent
        );
    }

    #[test]
    fn pictures_are_not_passed_on_and_long_text_is_cut_on_a_character_boundary() {
        let image = item(&json!({"type": "image", "data": "A".repeat(4000), "mimeType": "image/png"}));
        assert!(image["note"].as_str().unwrap().contains("not shown to you") && image.get("data").is_none());
        let long = item(&json!({"type": "text", "text": "é".repeat(150_000)}));
        let shown = long["text"].as_str().unwrap();
        assert!(shown.contains("[cut: ") && shown.len() < 300_000);
        assert_eq!(item(&json!({"type": "weird"}))["type"], "weird");
    }

    #[test]
    fn an_approval_card_gets_a_plain_sentence_with_argument_names_only() {
        let tool = McpTool::new("github", &def("create_issue", json!({})), ToolTrust::Ask, Arc::new(Nobody));
        let said = tool.describe_call(&json!({"title": "secret-ish", "body": "x"})).unwrap();
        assert!(
            said.contains("create_issue")
                && said.contains("github")
                && said.contains("body, title")
                && !said.contains("secret-ish")
        );
    }
}
