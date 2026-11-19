//! The adapter for Anthropic's Messages API (ADR-0009, ADR-0012).
//!
//! Choices that come from the API's own rules: the API version `2026-10-21` rejects `temperature`, `top_p` and `top_k`, so
//! they are never sent; current models always think, so `thinking` is omitted and depth is set with `output_config.effort`;
//! thinking blocks are bound to the conversation, so the raw assistant turn is kept and replayed unchanged.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use reqwest::Url;
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;
use zorix_core::{
    Effort, FinishReason, ModelError, ModelErrorKind, ModelGateway, ModelProfile, ModelRequest, ModelResponse,
    NativeTurn, NoSink, Role, TextSink, ToolCallId, ToolCallRequest, Usage,
};

use crate::adapter::{Discover, ToolNameMap, resolve_key, schema_instruction, simple_name_char};
use crate::catalog::Catalog;
use crate::http::{Header, HttpClient, classify_stream_error};
use crate::secrets::SecretStore;
use crate::sse::SseEvent;

/// The API version this adapter is written for.
pub const API_VERSION: &str = "2026-10-21";

/// The beta that lets the server re-run a declined request on a fallback model.
pub const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

/// Models that do not take `output_config.effort`.
const NO_EFFORT: &[&str] =
    &["claude-3", "claude-haiku-4", "claude-sonnet-4-5", "claude-opus-4-1", "claude-opus-4-0", "claude-sonnet-4-0"];

/// Models for which the server-side fallback is on by default.
const FALLBACK_MODELS: &[&str] = &["claude-fable-5", "claude-mythos-5", "claude-opus-5", "claude-sonnet-5-5"];

/// What can be tuned.
#[derive(Clone, Debug)]
pub struct AnthropicOptions {
    /// Effort when the task does not say. Tool loops do well at `high`.
    pub default_effort: Effort,
    /// Most output tokens when the task does not say (the model's own limit lowers it).
    pub default_max_tokens: u32,
    /// Ask the server to re-run a declined request on a fallback model, for the models that support it.
    pub server_fallbacks: bool,
}

impl Default for AnthropicOptions {
    fn default() -> Self {
        Self { default_effort: Effort::High, default_max_tokens: 64_000, server_fallbacks: true }
    }
}

/// An Anthropic provider instance.
pub struct AnthropicGateway {
    name: String,
    http: HttpClient,
    url: Url,
    secrets: Arc<dyn SecretStore>,
    key_name: String,
    catalog: Arc<Catalog>,
    options: AnthropicOptions,
    /// Set when the server turned out not to know the fallback beta, so it is not sent again.
    fallbacks_refused: AtomicBool,
}

impl AnthropicGateway {
    /// A gateway for the instance called `name`, at `base_url`, whose key is stored under `key_name`.
    pub fn new(
        name: &str,
        http: HttpClient,
        base_url: &Url,
        secrets: Arc<dyn SecretStore>,
        key_name: &str,
        catalog: Arc<Catalog>,
        options: AnthropicOptions,
    ) -> Result<Self, String> {
        let url = base_url
            .join("v1/messages")
            .or_else(|_| Url::parse(&format!("{}/v1/messages", base_url.as_str().trim_end_matches('/'))))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            name: name.into(),
            http,
            url,
            secrets,
            key_name: key_name.into(),
            catalog,
            options,
            fallbacks_refused: AtomicBool::new(false),
        })
    }
}

fn effort_name(effort: Effort) -> &'static str {
    match effort {
        Effort::Low => "low",
        Effort::Medium => "medium",
        Effort::High => "high",
        Effort::XHigh => "xhigh",
        Effort::Max => "max",
    }
}

fn takes_effort(model: &str) -> bool {
    !NO_EFFORT.iter().any(|prefix| model.starts_with(prefix))
}

fn supports_fallbacks(model: &str) -> bool {
    FALLBACK_MODELS.iter().any(|prefix| model.starts_with(prefix))
}

fn text_block(text: &str) -> Value {
    json!({"type": "text", "text": text})
}

/// Push blocks for `role`, merging with the previous message of the same role (the API wants alternation).
fn push(messages: &mut Vec<(&'static str, Vec<Value>)>, role: &'static str, blocks: Vec<Value>) {
    if blocks.is_empty() {
        return;
    }
    match messages.last_mut() {
        Some((last, existing)) if *last == role => existing.extend(blocks),
        _ => messages.push((role, blocks)),
    }
}

/// Build the request body for a model call. Pure, so it can be tested without a server.
pub fn build_request(
    request: &ModelRequest,
    options: &AnthropicOptions,
    max_output: Option<u64>,
    fallbacks: bool,
) -> (Value, ToolNameMap) {
    let names = ToolNameMap::new(&request.tools, simple_name_char);
    let replay_native = !request.history_edited;

    let mut system: Vec<Value> = Vec::new();
    let mut messages: Vec<(&'static str, Vec<Value>)> = Vec::new();
    let mut provider_ids: HashMap<ToolCallId, String> = HashMap::new();
    for message in &request.messages {
        match message.role {
            Role::System => {
                if !message.content.trim().is_empty() {
                    system.push(text_block(&message.content));
                }
            }
            Role::User => {
                if !message.content.trim().is_empty() {
                    push(&mut messages, "user", vec![text_block(&message.content)]);
                }
            }
            Role::Assistant => {
                for call in &message.tool_calls {
                    provider_ids.insert(call.id.clone(), call.provider_id.clone().unwrap_or_else(|| call.id.0.clone()));
                }
                let native = message
                    .native
                    .as_ref()
                    .filter(|n| replay_native && n.provider == "anthropic" && n.model == request.model);
                let blocks = match native.and_then(|n| n.content.as_array()) {
                    Some(blocks) => blocks.clone(),
                    None => {
                        let mut blocks = Vec::new();
                        if !message.content.trim().is_empty() {
                            blocks.push(text_block(&message.content));
                        }
                        for call in &message.tool_calls {
                            blocks.push(json!({
                                "type": "tool_use",
                                "id": provider_ids[&call.id],
                                "name": names.wire(&call.tool, simple_name_char),
                                "input": if call.arguments.is_object() { call.arguments.clone() } else { json!({}) },
                            }));
                        }
                        blocks
                    }
                };
                push(&mut messages, "assistant", blocks);
            }
            Role::Tool => {
                let id = message.tool_call_id.as_ref();
                let tool_use_id = id
                    .and_then(|id| provider_ids.get(id).cloned())
                    .or_else(|| id.map(|i| i.0.clone()))
                    .unwrap_or_default();
                let mut block = json!({"type": "tool_result", "tool_use_id": tool_use_id, "content": message.content});
                if message.is_error {
                    block["is_error"] = json!(true);
                }
                push(&mut messages, "user", vec![block]);
            }
        }
    }
    if let Some(schema) = &request.response_schema {
        system.push(text_block(&schema_instruction(schema)));
    }
    // The API wants tool results first in a user message.
    let messages: Vec<Value> = messages
        .into_iter()
        .map(|(role, mut blocks)| {
            if role == "user" {
                blocks.sort_by_key(|b| b["type"] != "tool_result");
            }
            json!({"role": role, "content": blocks})
        })
        .collect();

    let ceiling = max_output.unwrap_or(u64::from(options.default_max_tokens)).max(1);
    let max_tokens =
        u64::from(request.params.max_output_tokens.unwrap_or(options.default_max_tokens)).min(ceiling).max(1);

    let mut body = Map::new();
    body.insert("model".into(), json!(request.model));
    body.insert("max_tokens".into(), json!(max_tokens));
    body.insert("stream".into(), json!(true));
    body.insert("messages".into(), Value::Array(messages));
    // Caches the stable prefix. The server places the breakpoint on the last cacheable block.
    body.insert("cache_control".into(), json!({"type": "ephemeral"}));
    if !system.is_empty() {
        body.insert("system".into(), Value::Array(system));
    }
    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|t| json!({"name": names.wire(&t.id, simple_name_char), "description": t.description, "input_schema": t.input_schema}))
            .collect();
        body.insert("tools".into(), Value::Array(tools));
    }
    if takes_effort(&request.model) {
        let effort = request.params.effort.unwrap_or(options.default_effort);
        body.insert("output_config".into(), json!({"effort": effort_name(effort)}));
    }
    if fallbacks {
        body.insert("fallbacks".into(), json!("default"));
    }
    (Value::Object(body), names)
}

/// Builds a response out of the stream's events.
#[derive(Default)]
pub struct Aggregator {
    sink: Option<Arc<dyn TextSink>>,
    model: String,
    blocks: Vec<Option<Block>>,
    usage: Usage,
    stop_reason: Option<String>,
    refusal_category: Option<String>,
    stopped: bool,
    saw_fallback: bool,
}

struct Block {
    value: Value,
    text: String,
    json: String,
}

impl Aggregator {
    /// A new, empty aggregator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Pass the answer's text on as it arrives.
    pub fn with_sink(mut self, sink: Arc<dyn TextSink>) -> Self {
        self.sink = Some(sink);
        self
    }

    fn usage_from(&mut self, usage: &Value) {
        let read = |key: &str| usage[key].as_u64();
        if let Some(n) = read("input_tokens") {
            self.usage.input_tokens = n;
        }
        if let Some(n) = read("output_tokens") {
            self.usage.output_tokens = n;
        }
        if let Some(n) = read("cache_read_input_tokens") {
            self.usage.cache_read_tokens = n;
        }
        if let Some(n) = read("cache_creation_input_tokens") {
            self.usage.cache_write_tokens = n;
        }
    }

    /// Take one event. An `error` event becomes an error.
    pub fn on_event(&mut self, event: &SseEvent) -> Result<(), ModelError> {
        if event.data.trim().is_empty() {
            return Ok(());
        }
        let data: Value = serde_json::from_str(&event.data).map_err(|_| {
            ModelError::provider(ModelErrorKind::Unavailable, "anthropic: an event of the stream was not JSON.")
        })?;
        match data["type"].as_str().unwrap_or(event.event.as_deref().unwrap_or("")) {
            "message_start" => {
                self.model = data["message"]["model"].as_str().unwrap_or_default().to_string();
                self.usage_from(&data["message"]["usage"]);
            }
            "content_block_start" => {
                let index = data["index"].as_u64().unwrap_or(0) as usize;
                let block = data["content_block"].clone();
                if block["type"] == "fallback" {
                    self.saw_fallback = true;
                }
                if self.blocks.len() <= index {
                    self.blocks.resize_with(index + 1, || None);
                }
                self.blocks[index] = Some(Block { value: block, text: String::new(), json: String::new() });
            }
            "content_block_delta" => {
                let index = data["index"].as_u64().unwrap_or(0) as usize;
                if let Some(Some(block)) = self.blocks.get_mut(index) {
                    let delta = &data["delta"];
                    match delta["type"].as_str().unwrap_or("") {
                        "text_delta" => {
                            let piece = delta["text"].as_str().unwrap_or("");
                            block.text.push_str(piece);
                            if let (Some(sink), false) = (&self.sink, piece.is_empty()) {
                                sink.text(piece);
                            }
                        }
                        "thinking_delta" => block.text.push_str(delta["thinking"].as_str().unwrap_or("")),
                        "input_json_delta" => block.json.push_str(delta["partial_json"].as_str().unwrap_or("")),
                        "signature_delta" => block.value["signature"] = delta["signature"].clone(),
                        _ => {}
                    }
                }
            }
            "message_delta" => {
                if let Some(reason) = data["delta"]["stop_reason"].as_str() {
                    self.stop_reason = Some(reason.to_string());
                }
                if let Some(category) = data["delta"]["stop_details"]["category"].as_str() {
                    self.refusal_category = Some(category.to_string());
                }
                self.usage_from(&data["usage"]);
            }
            "message_stop" => self.stopped = true,
            "error" => return Err(classify_stream_error("anthropic", &data["error"])),
            _ => {} // ping, content_block_stop and anything new
        }
        Ok(())
    }

    /// The finished response. Fails if the stream ended before `message_stop`, or if the provider declined.
    pub fn finish(self, requested_model: &str, names: &ToolNameMap) -> Result<ModelResponse, ModelError> {
        if !self.stopped {
            return Err(ModelError::provider(
                ModelErrorKind::Unavailable,
                "anthropic: the response ended before it was complete.",
            ));
        }
        if self.stop_reason.as_deref() == Some("refusal") {
            let category = self.refusal_category.map(|c| format!(" (category: {c})")).unwrap_or_default();
            return Err(ModelError::provider(
                ModelErrorKind::Refused,
                format!("anthropic: the model declined to answer{category}."),
            ));
        }
        let mut content: Vec<Value> = Vec::new();
        let mut text = String::new();
        let mut calls: Vec<ToolCallRequest> = Vec::new();
        for block in self.blocks.into_iter().flatten() {
            let mut value = block.value;
            match value["type"].as_str().unwrap_or("") {
                "text" => {
                    text.push_str(&block.text);
                    value["text"] = json!(block.text);
                }
                "thinking" => value["thinking"] = json!(block.text),
                "tool_use" => {
                    let arguments = if block.json.trim().is_empty() {
                        json!({})
                    } else {
                        serde_json::from_str(&block.json).unwrap_or_else(|_| Value::String(block.json.clone()))
                    };
                    value["input"] = if arguments.is_object() { arguments.clone() } else { json!({}) };
                    calls.push(ToolCallRequest {
                        id: ToolCallId(String::new()),
                        provider_id: value["id"].as_str().map(str::to_string),
                        tool: names.original(value["name"].as_str().unwrap_or("")),
                        arguments,
                    });
                }
                _ => {}
            }
            content.push(value);
        }
        let finish = match self.stop_reason.as_deref() {
            Some("tool_use") => FinishReason::ToolCalls,
            Some("max_tokens") | Some("model_context_window_exceeded") => FinishReason::Length,
            _ if !calls.is_empty() => FinishReason::ToolCalls,
            _ => FinishReason::Stop,
        };
        let model = if self.model.is_empty() { requested_model.to_string() } else { self.model };
        // A turn that contains a fallback marker is not replayed: the portable form is always valid.
        let native = (!self.saw_fallback).then(|| NativeTurn {
            provider: "anthropic".into(),
            model: model.clone(),
            content: Value::Array(content),
        });
        let mut response = ModelResponse::new(model, text, calls, Usage { model_calls: 1, ..self.usage }, finish);
        response.provider = Some("anthropic".into());
        response.native = native;
        Ok(response)
    }
}

#[async_trait]
impl Discover for AnthropicGateway {
    async fn discover(&self, cancel: &CancellationToken) -> Result<Vec<crate::catalog::ModelInfo>, ModelError> {
        let key = resolve_key(&self.secrets, "anthropic", &self.key_name).await?;
        let mut url = self.url.clone();
        url.set_path("/v1/models");
        url.set_query(Some("limit=100"));
        let headers = [("x-api-key", Header::Secret(&key)), ("anthropic-version", Header::Plain(API_VERSION))];
        let list = self.http.get_json(&url, &headers, cancel).await?;
        Ok(list["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| {
                let id = m["id"].as_str()?.to_string();
                // What the catalogue already knows (with its price) wins over what the list reports.
                if let Some(known) = self.catalog.get(&self.name, &id) {
                    return Some(known.clone());
                }
                let context = m["max_input_tokens"].as_u64().filter(|n| *n > 0);
                let window = context.unwrap_or(200_000);
                Some(crate::catalog::ModelInfo {
                    provider: self.name.clone(),
                    display_name: m["display_name"].as_str().unwrap_or(&id).to_string(),
                    max_output: m["max_tokens"].as_u64().filter(|n| *n > 0).unwrap_or(8_192).min(window),
                    context_window: window,
                    tools: true,
                    vision: true,
                    thinking: true,
                    latency: crate::catalog::Latency::Standard,
                    tier: 3,
                    price: None,
                    assumed: context.is_none(),
                    id,
                })
            })
            .collect())
    }
}

#[async_trait]
impl ModelGateway for AnthropicGateway {
    async fn generate(&self, request: ModelRequest, cancel: CancellationToken) -> Result<ModelResponse, ModelError> {
        self.generate_live(request, cancel, Arc::new(NoSink)).await
    }

    async fn generate_live(
        &self,
        request: ModelRequest,
        cancel: CancellationToken,
        sink: Arc<dyn TextSink>,
    ) -> Result<ModelResponse, ModelError> {
        let key = resolve_key(&self.secrets, "anthropic", &self.key_name).await?;
        let info = self.catalog.get(&self.name, &request.model);
        let max_output = info.map(|m| m.max_output);
        let mut with_fallbacks = self.options.server_fallbacks
            && supports_fallbacks(&request.model)
            && !self.fallbacks_refused.load(Ordering::Relaxed);
        loop {
            let (body, names) = build_request(&request, &self.options, max_output, with_fallbacks);
            let mut headers: Vec<(&str, Header)> =
                vec![("x-api-key", Header::Secret(&key)), ("anthropic-version", Header::Plain(API_VERSION))];
            if with_fallbacks {
                headers.push(("anthropic-beta", Header::Plain(FALLBACK_BETA)));
            }
            let response = match self.http.post_json(&self.url, &headers, &body, true, &cancel).await {
                Ok(response) => response,
                Err(error) => {
                    // An account or a deployment that does not know the beta: say so once and carry on without it.
                    let text = error.to_string();
                    if with_fallbacks
                        && error.kind() == ModelErrorKind::Invalid
                        && (text.contains("anthropic-beta") || text.contains("fallbacks"))
                    {
                        self.fallbacks_refused.store(true, Ordering::Relaxed);
                        with_fallbacks = false;
                        continue;
                    }
                    return Err(error);
                }
            };
            let mut reader = self.http.sse(response);
            let mut aggregator = Aggregator::new().with_sink(sink.clone());
            while let Some(event) = reader.next(&cancel).await? {
                aggregator.on_event(&event)?;
            }
            let mut result = aggregator.finish(&request.model, &names)?;
            result.usage.cost_micros = self
                .catalog
                .cost_micros(&self.name, &result.model, &result.usage)
                .or_else(|| self.catalog.cost_micros(&self.name, &request.model, &result.usage))
                .unwrap_or(0);
            return Ok(result);
        }
    }

    fn describe(&self, model: &str) -> Option<ModelProfile> {
        self.catalog.get(&self.name, model).map(|m| m.profile())
    }
}
