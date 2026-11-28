//! The MCP client: finds out which era a server speaks, lists its tools and calls them (ADR-0018).
//!
//! It never answers a server's request for model access, user input or folders. A tool call that needs more input fails
//! with an explanation instead.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use zorix_core::integrations::MAX_TOOLS_PER_SERVER;

use crate::error::{METHOD_NOT_FOUND, McpError, UNSUPPORTED_VERSION};
use crate::headers::{Mirror, mirrors as header_mirrors, param_headers};
use crate::protocol::{Era, LEGACY_VERSIONS, MODERN_VERSION, client_info, with_modern_meta};
use crate::transport::{ServerEvent, Transport};

/// How long to wait for the probe before assuming a server speaks the legacy protocol.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(8);
/// How long the handshake may take once the era is known.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
/// The most pages of tools followed.
const MAX_PAGES: usize = 50;
/// Longest tool name accepted.
pub const MAX_TOOL_NAME: usize = 128;

/// What a server says about itself. Self-reported: for display, never for security decisions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServerInfo {
    /// Its name.
    pub name: Option<String>,
    /// Its version.
    pub version: Option<String>,
    /// Its advice for the model.
    pub instructions: Option<String>,
    /// What it supports.
    pub capabilities: Value,
}

/// A tool as a server lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct McpToolDef {
    /// Its name at the server.
    pub name: String,
    /// A name for people.
    pub title: Option<String>,
    /// What it does, written by the server.
    pub description: String,
    /// JSON Schema of the arguments.
    pub input_schema: Value,
    /// JSON Schema of the result.
    pub output_schema: Option<Value>,
    /// The server's hints. Untrusted.
    pub annotations: Value,
}

impl McpToolDef {
    /// The definition as it is stored and shown in a review.
    pub fn definition(&self) -> Value {
        json!({
            "name": self.name,
            "title": self.title,
            "description": self.description,
            "inputSchema": self.input_schema,
            "outputSchema": self.output_schema,
            "annotations": self.annotations,
        })
    }

    /// The hash that pins it. Everything the model or the trust rules read is in it, annotations included, so a server
    /// that flips a hint after approval triggers a review.
    pub fn hash(&self) -> String {
        zorix_core::canonical_hash(&self.definition())
    }
}

/// The tools of a server, and the ones that were left out with the reason.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolListing {
    /// Valid tools.
    pub tools: Vec<McpToolDef>,
    /// `(name, reason)` of tools that were not accepted.
    pub skipped: Vec<(String, String)>,
}

/// What a tool call returned.
#[derive(Clone, Debug, PartialEq)]
pub struct CallOutput {
    /// Content items as the server sent them.
    pub content: Vec<Value>,
    /// Structured content, when there is some.
    pub structured: Option<Value>,
    /// The server says the call failed (the text says why).
    pub is_error: bool,
}

/// Tool names: 1 to 128 of letters, digits, `_`, `-` and `.`.
pub fn valid_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_TOOL_NAME
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

fn parse_tool(raw: &Value) -> Result<McpToolDef, (String, String)> {
    let name = raw.get("name").and_then(Value::as_str).unwrap_or("").to_string();
    let skip = |reason: &str| (name.clone(), reason.to_string());
    if !valid_tool_name(&name) {
        return Err(skip("its name is not 1 to 128 letters, digits, '_', '-' or '.'"));
    }
    let input_schema = raw.get("inputSchema").cloned().unwrap_or_else(|| json!({"type": "object"}));
    if !input_schema.is_object() {
        return Err(skip("its input schema is not an object"));
    }
    if jsonschema::validator_for(&input_schema).is_err() {
        return Err(skip("its input schema is not a valid JSON Schema"));
    }
    let output_schema = raw.get("outputSchema").filter(|s| !s.is_null()).cloned();
    if output_schema.as_ref().is_some_and(|s| jsonschema::validator_for(s).is_err()) {
        return Err(skip("its output schema is not a valid JSON Schema"));
    }
    Ok(McpToolDef {
        name: name.clone(),
        title: raw.get("title").and_then(Value::as_str).map(str::to_string),
        description: raw.get("description").and_then(Value::as_str).unwrap_or("").to_string(),
        input_schema,
        output_schema,
        annotations: raw.get("annotations").filter(|a| a.is_object()).cloned().unwrap_or_else(|| json!({})),
    })
}

/// A connection to one server whose era is known.
pub struct McpClient {
    transport: Arc<dyn Transport>,
    era: Era,
    info: ServerInfo,
    /// Parameters each tool wants mirrored into HTTP headers, learned when the tools are listed.
    mirrors: Mutex<HashMap<String, Vec<Mirror>>>,
    /// What the server does besides answering, plus what the client adds (a stream that came back after a gap).
    events: broadcast::Sender<ServerEvent>,
    /// Ends the background tasks of this client.
    stop: CancellationToken,
    /// Signalled when the server acknowledged the subscription to changes.
    acked: Arc<tokio::sync::Notify>,
    /// A subscription was asked for.
    listening: bool,
}

impl McpClient {
    /// Find out what the server speaks and make it ready.
    pub async fn connect(transport: Arc<dyn Transport>, cancel: &CancellationToken) -> Result<Self, McpError> {
        Self::connect_with(transport, cancel, PROBE_TIMEOUT, HANDSHAKE_TIMEOUT).await
    }

    /// The same with chosen timeouts (for tests).
    pub async fn connect_with(
        transport: Arc<dyn Transport>,
        cancel: &CancellationToken,
        probe: Duration,
        handshake: Duration,
    ) -> Result<Self, McpError> {
        match transport.request("server/discover", with_modern_meta(Value::Null), probe, cancel).await {
            Ok(result) => {
                let versions: Vec<&str> = result
                    .get("supportedVersions")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();
                if versions.contains(&MODERN_VERSION) {
                    return Ok(Self::ready(transport, Era::Modern, info_from(&result)).settle().await);
                }
                if versions.is_empty() {
                    return Self::legacy(transport, cancel, handshake).await;
                }
                if let Some(version) = LEGACY_VERSIONS.iter().find(|v| versions.contains(v)) {
                    return Self::legacy_at(transport, cancel, handshake, version).await;
                }
                Err(McpError::Protocol(format!(
                    "the server speaks {} and ZORIX speaks {MODERN_VERSION} and {}",
                    versions.join(", "),
                    LEGACY_VERSIONS.join(", ")
                )))
            }
            // A recognised modern error: the server is modern but wants another version.
            Err(McpError::Rpc { code, data, .. }) if code == UNSUPPORTED_VERSION => {
                let supported: Vec<String> = data
                    .as_ref()
                    .and_then(|d| d.get("supported"))
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                match LEGACY_VERSIONS.iter().find(|v| supported.iter().any(|s| s == *v)) {
                    Some(version) => Self::legacy_at(transport, cancel, handshake, version).await,
                    None => Err(McpError::Protocol(format!(
                        "the server supports {}, which ZORIX does not speak (it speaks {MODERN_VERSION} and {})",
                        supported.join(", "),
                        LEGACY_VERSIONS.join(", ")
                    ))),
                }
            }
            // The process died, or something is broken for good: do not guess.
            Err(
                e @ (McpError::Closed
                | McpError::Spawn(_)
                | McpError::TooLarge { .. }
                | McpError::Cancelled
                | McpError::NeedsLogin(_)),
            ) => Err(e),
            // Any other answer, or none: a legacy server (the fallback must not depend on one error code).
            Err(_) => Self::legacy(transport, cancel, handshake).await,
        }
    }

    async fn legacy(
        transport: Arc<dyn Transport>,
        cancel: &CancellationToken,
        handshake: Duration,
    ) -> Result<Self, McpError> {
        Self::legacy_at(transport, cancel, handshake, LEGACY_VERSIONS[0]).await
    }

    async fn legacy_at(
        transport: Arc<dyn Transport>,
        cancel: &CancellationToken,
        handshake: Duration,
        version: &str,
    ) -> Result<Self, McpError> {
        let params = json!({"protocolVersion": version, "capabilities": {}, "clientInfo": client_info()});
        let result = transport.request("initialize", params, handshake, cancel).await?;
        let agreed = result.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
        if !LEGACY_VERSIONS.contains(&agreed) {
            return Err(McpError::Protocol(format!(
                "the server answered with protocol version “{agreed}”, which ZORIX does not speak (it speaks {})",
                LEGACY_VERSIONS.join(", ")
            )));
        }
        transport.notify("notifications/initialized", Value::Null).await?;
        Ok(Self::ready(transport, Era::Legacy { version: agreed.to_string() }, info_from(&result)).settle().await)
    }

    fn ready(transport: Arc<dyn Transport>, era: Era, info: ServerInfo) -> Self {
        let (events, _) = broadcast::channel(256);
        let stop = CancellationToken::new();
        let listening = era == Era::Modern && info.capabilities["tools"]["listChanged"] == true;
        let client = Self {
            transport,
            era,
            info,
            mirrors: Mutex::new(HashMap::new()),
            events,
            stop,
            acked: Arc::new(tokio::sync::Notify::new()),
            listening,
        };
        client.forward_events();
        if listening {
            client.listen_for_changes();
        }
        client
    }

    /// Wait (briefly) until the server confirms it will announce changes, so none is missed between listing and listening.
    async fn settle(self) -> Self {
        if self.listening {
            let _ = tokio::time::timeout(Duration::from_secs(2), self.acked.notified()).await;
        }
        self
    }

    /// Pass on what the transport reports, so the client can add to it.
    fn forward_events(&self) {
        let mut from = self.transport.subscribe();
        let to = self.events.clone();
        let stop = self.stop.clone();
        let acked = self.acked.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop.cancelled() => return,
                    event = from.recv() => match event {
                        Ok(event) => {
                            if matches!(&event, ServerEvent::Notification { method, .. } if method == "notifications/subscriptions/acknowledged") {
                                acked.notify_one();
                            }
                            let _ = to.send(event);
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(broadcast::error::RecvError::Closed) => return,
                    },
                }
            }
        });
    }

    /// Modern servers announce changes only to a client that asked (`subscriptions/listen`). The request stays open; when it
    /// ends without the client closing it, it is opened again, and the tools are looked at again in case something was missed.
    fn listen_for_changes(&self) {
        let transport = self.transport.clone();
        let stop = self.stop.clone();
        let events = self.events.clone();
        tokio::spawn(async move {
            let mut wait = Duration::from_secs(1);
            let mut first = true;
            loop {
                if stop.is_cancelled() || !transport.alive() {
                    return;
                }
                if !first {
                    let _ = events.send(ServerEvent::Notification {
                        method: "notifications/tools/list_changed".into(),
                        params: Value::Null,
                    });
                }
                first = false;
                let params = with_modern_meta(json!({"notifications": {"toolsListChanged": true}}));
                let started = std::time::Instant::now();
                match transport
                    .request("subscriptions/listen", params, Duration::from_secs(30 * 24 * 3600), &stop)
                    .await
                {
                    Err(McpError::Cancelled | McpError::Closed) => return,
                    Err(McpError::Rpc { code, .. }) if code == METHOD_NOT_FOUND => return,
                    _ => {}
                }
                if started.elapsed() > Duration::from_secs(30) {
                    wait = Duration::from_secs(1);
                }
                tokio::select! {
                    _ = tokio::time::sleep(wait) => {}
                    _ = stop.cancelled() => return,
                }
                wait = (wait * 2).min(Duration::from_secs(30));
            }
        });
    }

    /// Which era the server speaks.
    pub fn era(&self) -> &Era {
        &self.era
    }

    /// What the server says about itself.
    pub fn info(&self) -> &ServerInfo {
        &self.info
    }

    /// What the server does besides answering.
    pub fn events(&self) -> broadcast::Receiver<ServerEvent> {
        self.events.subscribe()
    }

    /// False once the connection has ended.
    pub fn alive(&self) -> bool {
        self.transport.alive()
    }

    /// The server's error stream, redacted.
    pub fn log(&self) -> Vec<String> {
        self.transport.log()
    }

    /// End the connection.
    pub async fn close(&self) {
        self.stop.cancel();
        self.transport.close().await;
    }

    /// Wait (briefly) for the last of the server's error stream, so [`log`](Self::log) is complete.
    pub async fn drained(&self) {
        self.transport.drained().await;
    }

    async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<Value, McpError> {
        self.request_with(method, params, &[], timeout, cancel).await
    }

    async fn request_with(
        &self,
        method: &str,
        params: Value,
        headers: &[(String, String)],
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<Value, McpError> {
        let params = match self.era {
            Era::Modern => with_modern_meta(params),
            Era::Legacy { .. } => params,
        };
        self.transport.request_with(method, params, headers, timeout, cancel).await
    }

    /// The tools the server lists now.
    pub async fn list_tools(&self, cancel: &CancellationToken) -> Result<ToolListing, McpError> {
        let mut listing = ToolListing::default();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let params = match &cursor {
                Some(cursor) => json!({"cursor": cursor}),
                None => json!({}),
            };
            let result = self.request("tools/list", params, Duration::from_secs(30), cancel).await?;
            let tools = result
                .get("tools")
                .and_then(Value::as_array)
                .ok_or_else(|| McpError::Protocol("tools/list did not return a list of tools".into()))?;
            for raw in tools {
                match parse_tool(raw) {
                    Ok(tool) if listing.tools.iter().any(|t| t.name == tool.name) => {
                        listing.skipped.push((tool.name, "the server lists it twice".into()));
                    }
                    Ok(tool) => match header_mirrors(&tool.input_schema) {
                        Ok(found) => {
                            self.mirrors.lock().insert(tool.name.clone(), found);
                            listing.tools.push(tool);
                        }
                        Err(reason) => listing.skipped.push((tool.name, reason)),
                    },
                    Err(skipped) => listing.skipped.push(skipped),
                }
                if listing.tools.len() > MAX_TOOLS_PER_SERVER {
                    return Err(McpError::Protocol(format!(
                        "the server lists more than {MAX_TOOLS_PER_SERVER} tools, which is more than ZORIX keeps"
                    )));
                }
            }
            cursor = result.get("nextCursor").and_then(Value::as_str).filter(|c| !c.is_empty()).map(str::to_string);
            if cursor.is_none() {
                return Ok(listing);
            }
        }
        Err(McpError::Protocol(format!("the server has more than {MAX_PAGES} pages of tools")))
    }

    /// Call a tool.
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Value,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<CallOutput, McpError> {
        let headers = self.mirrors.lock().get(name).map(|m| param_headers(m, &arguments)).unwrap_or_default();
        let result = self
            .request_with("tools/call", json!({"name": name, "arguments": arguments}), &headers, timeout, cancel)
            .await?;
        if result.get("resultType").and_then(Value::as_str) == Some("input_required") {
            let wanted: Vec<String> = result
                .get("inputRequests")
                .and_then(Value::as_object)
                .map(|requests| {
                    requests
                        .values()
                        .filter_map(|r| r.get("method").and_then(Value::as_str))
                        .map(|m| m.split('/').next().unwrap_or(m).to_string())
                        .collect()
                })
                .unwrap_or_default();
            return Err(McpError::Refused(format!(
                "{name} needs more input from you or access to a model ({}). ZORIX does not let servers ask for that, so the call was not completed.",
                if wanted.is_empty() { "unspecified".to_string() } else { wanted.join(", ") }
            )));
        }
        let content = result.get("content").and_then(Value::as_array).cloned().unwrap_or_default();
        let structured = result.get("structuredContent").filter(|s| !s.is_null()).cloned();
        if content.is_empty() && structured.is_none() && result.get("isError").is_none() {
            return Err(McpError::Protocol("tools/call returned neither content nor structured content".into()));
        }
        Ok(CallOutput {
            content,
            structured,
            is_error: result.get("isError").and_then(Value::as_bool).unwrap_or(false),
        })
    }
}

fn info_from(result: &Value) -> ServerInfo {
    let meta = result.get("_meta").and_then(|m| m.get("io.modelcontextprotocol/serverInfo"));
    let info = meta.or_else(|| result.get("serverInfo"));
    ServerInfo {
        name: info.and_then(|i| i.get("name")).and_then(Value::as_str).map(str::to_string),
        version: info.and_then(|i| i.get("version")).and_then(Value::as_str).map(str::to_string),
        instructions: result.get("instructions").and_then(Value::as_str).map(str::to_string),
        capabilities: result.get("capabilities").cloned().unwrap_or_else(|| json!({})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_follow_the_spec_and_bad_tools_are_skipped_with_a_reason() {
        for ok in ["a", "get_weather", "DATA_EXPORT_v2", "admin.tools.list", "a-b"] {
            assert!(valid_tool_name(ok), "{ok}");
        }
        for bad in ["", "has space", "comma,name", "naïve", &"x".repeat(129)] {
            assert!(!valid_tool_name(bad), "{bad}");
        }
        assert!(parse_tool(&json!({"name": "ok", "inputSchema": {"type": "object"}})).is_ok());
        assert!(parse_tool(&json!({"name": "bad name"})).is_err());
        assert!(parse_tool(&json!({"name": "x", "inputSchema": "nope"})).is_err());
        assert!(parse_tool(&json!({"name": "x", "inputSchema": {"type": "wrong"}})).is_err());
        assert!(parse_tool(&json!({"name": "x", "outputSchema": {"type": 5}})).is_err());
    }

    #[test]
    fn the_pin_covers_everything_the_model_and_the_trust_rules_read() {
        let base = parse_tool(&json!({"name": "t", "description": "d", "inputSchema": {"type": "object"}, "annotations": {"readOnlyHint": true}})).unwrap();
        let mut worded = base.clone();
        worded.description.push('!');
        let mut hinted = base.clone();
        hinted.annotations = json!({"readOnlyHint": false});
        let mut titled = base.clone();
        titled.title = Some("T".into());
        for changed in [worded, hinted, titled] {
            assert_ne!(base.hash(), changed.hash());
        }
        assert_eq!(base.hash(), base.clone().hash());
    }
}
