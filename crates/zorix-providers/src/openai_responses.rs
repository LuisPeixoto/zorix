//! The adapter for OpenAI's Responses API, the API current OpenAI models need for tool calling.
//!
//! The conversation is sent statelessly (`store: false`). Reasoning items that come back next to tool calls are kept in the
//! assistant message's native turn and sent back unchanged, which is what the API asks for (ADR-0012).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use reqwest::Url;
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;
use zorix_core::{
    Effort, FinishReason, ModelError, ModelErrorKind, ModelGateway, ModelProfile, ModelRequest, ModelResponse,
    NativeTurn, Role, ToolCallId, ToolCallRequest, Usage,
};

use crate::adapter::{Discover, ToolNameMap, parse_openai_models, resolve_key, schema_instruction, simple_name_char};
use crate::catalog::Catalog;
use crate::http::{Header, HttpClient};
use crate::secrets::SecretStore;
use crate::sse::SseEvent;

/// What can be tuned.
#[derive(Clone, Debug)]
pub struct OpenAiOptions {
    /// Model id prefixes that reason before answering. For these, effort is sent and reasoning items are requested.
    pub reasoning_prefixes: Vec<String>,
    /// Effort when the task does not say.
    pub default_effort: Effort,
    /// Let OpenAI keep the conversation on its servers (`store: true`). Off by default: ZORIX keeps the record.
    pub store: bool,
}

impl Default for OpenAiOptions {
    fn default() -> Self {
        Self {
            reasoning_prefixes: ["o1", "o3", "o4", "gpt-5", "gpt-6"].into_iter().map(String::from).collect(),
            default_effort: Effort::High,
            store: false,
        }
    }
}

/// An OpenAI (Responses API) provider instance.
pub struct OpenAiResponsesGateway {
    name: String,
    http: HttpClient,
    url: Url,
    secrets: Arc<dyn SecretStore>,
    key_name: String,
    catalog: Arc<Catalog>,
    options: OpenAiOptions,
    /// Set when the API refused our reasoning items, so later calls send the portable form.
    replay_refused: AtomicBool,
}

impl OpenAiResponsesGateway {
    /// A gateway for the instance called `name` at `base_url` (such as `https://api.openai.com/v1`).
    pub fn new(
        name: &str,
        http: HttpClient,
        base_url: &Url,
        secrets: Arc<dyn SecretStore>,
        key_name: &str,
        catalog: Arc<Catalog>,
        options: OpenAiOptions,
    ) -> Result<Self, String> {
        let url =
            Url::parse(&format!("{}/responses", base_url.as_str().trim_end_matches('/'))).map_err(|e| e.to_string())?;
        Ok(Self {
            name: name.into(),
            http,
            url,
            secrets,
            key_name: key_name.into(),
            catalog,
            options,
            replay_refused: AtomicBool::new(false),
        })
    }
}

fn effort_name(effort: Effort) -> &'static str {
    match effort {
        Effort::Low => "low",
        Effort::Medium => "medium",
        Effort::High | Effort::XHigh | Effort::Max => "high",
    }
}

fn is_reasoning(model: &str, options: &OpenAiOptions) -> bool {
    options.reasoning_prefixes.iter().any(|prefix| model.starts_with(prefix.as_str()))
}

/// Build the request body. Pure, so it can be tested without a server.
pub fn build_request(request: &ModelRequest, options: &OpenAiOptions, replay_native: bool) -> (Value, ToolNameMap) {
    let names = ToolNameMap::new(&request.tools, simple_name_char);
    let replay_native = replay_native && !request.history_edited;

    let mut instructions: Vec<String> = Vec::new();
    let mut input: Vec<Value> = Vec::new();
    let mut provider_ids: std::collections::HashMap<ToolCallId, String> = std::collections::HashMap::new();
    for message in &request.messages {
        match message.role {
            Role::System => {
                if !message.content.trim().is_empty() {
                    instructions.push(message.content.clone());
                }
            }
            Role::User => {
                if !message.content.trim().is_empty() {
                    input.push(json!({"role": "user", "content": [{"type": "input_text", "text": message.content}]}));
                }
            }
            Role::Assistant => {
                for call in &message.tool_calls {
                    provider_ids.insert(call.id.clone(), call.provider_id.clone().unwrap_or_else(|| call.id.0.clone()));
                }
                let native = message
                    .native
                    .as_ref()
                    .filter(|n| replay_native && n.provider == "openai" && n.model == request.model);
                if let Some(items) = native.and_then(|n| n.content.as_array()) {
                    input.extend(items.iter().cloned());
                } else {
                    if !message.content.trim().is_empty() {
                        input.push(
                            json!({"role": "assistant", "content": [{"type": "output_text", "text": message.content}]}),
                        );
                    }
                    for call in &message.tool_calls {
                        // No `id`: the portable form refers to nothing the server stored.
                        input.push(json!({
                            "type": "function_call",
                            "call_id": provider_ids[&call.id],
                            "name": names.wire(&call.tool, simple_name_char),
                            "arguments": call.arguments.to_string(),
                        }));
                    }
                }
            }
            Role::Tool => {
                let id = message.tool_call_id.as_ref();
                let call_id = id
                    .and_then(|id| provider_ids.get(id).cloned())
                    .or_else(|| id.map(|i| i.0.clone()))
                    .unwrap_or_default();
                input.push(json!({"type": "function_call_output", "call_id": call_id, "output": message.content}));
            }
        }
    }
    if let Some(schema) = &request.response_schema {
        instructions.push(schema_instruction(schema));
    }

    let mut body = Map::new();
    body.insert("model".into(), json!(request.model));
    body.insert("input".into(), Value::Array(input));
    body.insert("stream".into(), json!(true));
    body.insert("store".into(), json!(options.store));
    if !instructions.is_empty() {
        body.insert("instructions".into(), json!(instructions.join("\n\n")));
    }
    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|t| json!({"type": "function", "name": names.wire(&t.id, simple_name_char), "description": t.description, "parameters": t.input_schema, "strict": false}))
            .collect();
        body.insert("tools".into(), Value::Array(tools));
    }
    if let Some(max) = request.params.max_output_tokens {
        body.insert("max_output_tokens".into(), json!(max));
    }
    if is_reasoning(&request.model, options) {
        let effort = request.params.effort.unwrap_or(options.default_effort);
        body.insert("reasoning".into(), json!({"effort": effort_name(effort)}));
        // Without server-side storage the reasoning comes back encrypted, to be sent back with the tool outputs.
        body.insert("include".into(), json!(["reasoning.encrypted_content"]));
    }
    (Value::Object(body), names)
}

fn error_kind(code: &str, message: &str) -> ModelErrorKind {
    let lower = format!("{code} {message}").to_lowercase();
    if code.contains("rate_limit") || lower.contains("rate limit") {
        ModelErrorKind::RateLimited
    } else if code.contains("context_length") || lower.contains("context length") || lower.contains("maximum context") {
        ModelErrorKind::ContextTooLong
    } else if code.contains("invalid_api_key") || code.contains("authentication") || code.contains("permission") {
        ModelErrorKind::Auth
    } else if code.contains("not_found") || code.contains("model_not_found") {
        ModelErrorKind::NotFound
    } else if code.contains("content_filter") || code.contains("refus") {
        ModelErrorKind::Refused
    } else if code.contains("invalid") && !code.contains("server") {
        ModelErrorKind::Invalid
    } else {
        ModelErrorKind::Unavailable
    }
}

/// Builds a response from the stream. The stream ends with `response.completed`, which carries the whole answer, so the
/// deltas before it are not needed.
#[derive(Default)]
pub struct ResponsesAggregator {
    done: Option<Value>,
}

impl ResponsesAggregator {
    /// A new aggregator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Take one event.
    pub fn on_event(&mut self, event: &SseEvent) -> Result<(), ModelError> {
        let data = event.data.trim();
        if data.is_empty() || data == "[DONE]" {
            return Ok(());
        }
        let value: Value = serde_json::from_str(data).map_err(|_| {
            ModelError::provider(ModelErrorKind::Unavailable, "openai: an event of the stream was not JSON.")
        })?;
        match value["type"].as_str().unwrap_or("") {
            "response.completed" | "response.incomplete" => self.done = Some(value["response"].clone()),
            "response.failed" => {
                let error = &value["response"]["error"];
                return Err(stream_error(&error["code"], &error["message"]));
            }
            "error" => return Err(stream_error(&value["code"], &value["message"])),
            _ => {}
        }
        Ok(())
    }

    /// The finished response, or why there is none.
    pub fn finish(self, requested_model: &str, names: &ToolNameMap) -> Result<ModelResponse, ModelError> {
        let response = self.done.ok_or_else(|| {
            ModelError::provider(ModelErrorKind::Unavailable, "openai: the response ended before it was complete.")
        })?;
        let items = response["output"].as_array().cloned().unwrap_or_default();
        let mut text = String::new();
        let mut refusal = String::new();
        let mut calls: Vec<ToolCallRequest> = Vec::new();
        for item in &items {
            match item["type"].as_str().unwrap_or("") {
                "message" => {
                    for part in item["content"].as_array().into_iter().flatten() {
                        match part["type"].as_str().unwrap_or("") {
                            "output_text" => text.push_str(part["text"].as_str().unwrap_or("")),
                            "refusal" => refusal.push_str(part["refusal"].as_str().unwrap_or("")),
                            _ => {}
                        }
                    }
                }
                "function_call" => {
                    let raw = item["arguments"].as_str().unwrap_or("");
                    let arguments = if raw.trim().is_empty() {
                        json!({})
                    } else {
                        serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
                    };
                    calls.push(ToolCallRequest {
                        id: ToolCallId(String::new()),
                        provider_id: item["call_id"].as_str().map(str::to_string),
                        tool: names.original(item["name"].as_str().unwrap_or("")),
                        arguments,
                    });
                }
                _ => {}
            }
        }
        if text.is_empty() && calls.is_empty() && !refusal.is_empty() {
            return Err(ModelError::provider(
                ModelErrorKind::Refused,
                format!("openai: the model declined to answer: {}", crate::http::shorten(&refusal, 200)),
            ));
        }
        let incomplete = response["status"] == "incomplete";
        let finish = if !calls.is_empty() {
            FinishReason::ToolCalls
        } else if incomplete {
            FinishReason::Length
        } else {
            FinishReason::Stop
        };
        let usage = &response["usage"];
        let cached = usage["input_tokens_details"]["cached_tokens"].as_u64().unwrap_or(0);
        let total_input = usage["input_tokens"].as_u64().unwrap_or(0);
        let usage = Usage {
            input_tokens: total_input.saturating_sub(cached),
            cache_read_tokens: cached,
            output_tokens: usage["output_tokens"].as_u64().unwrap_or(0),
            reasoning_tokens: usage["output_tokens_details"]["reasoning_tokens"].as_u64().unwrap_or(0),
            model_calls: 1,
            ..Usage::default()
        };
        let model = response["model"].as_str().filter(|m| !m.is_empty()).unwrap_or(requested_model).to_string();
        let mut result = ModelResponse::new(model.clone(), text, calls, usage, finish);
        result.provider = Some("openai".into());
        result.native = Some(NativeTurn { provider: "openai".into(), model, content: Value::Array(items) });
        Ok(result)
    }
}

fn stream_error(code: &Value, message: &Value) -> ModelError {
    let code = code.as_str().unwrap_or("");
    let message = crate::http::shorten(&zorix_core::redact_text(message.as_str().unwrap_or("")), 240);
    ModelError::provider(
        error_kind(code, &message),
        format!("openai: the stream reported an error ({code}): {message}"),
    )
}

#[async_trait]
impl Discover for OpenAiResponsesGateway {
    async fn discover(&self, cancel: &CancellationToken) -> Result<Vec<crate::catalog::ModelInfo>, ModelError> {
        let key = Some(resolve_key(&self.secrets, "openai", &self.key_name).await?);
        let mut url = self.url.clone();
        let path =
            url.path().trim_end_matches('/').rsplit_once('/').map(|(head, _)| head.to_string()).unwrap_or_default();
        url.set_path(&format!("{path}/models"));
        let headers: Vec<(&str, Header)> = key.iter().map(|k| ("authorization", Header::Bearer(k))).collect();
        let list = self.http.get_json(&url, &headers, cancel).await?;
        Ok(parse_openai_models("openai", &list))
    }
}

#[async_trait]
impl ModelGateway for OpenAiResponsesGateway {
    async fn generate(&self, request: ModelRequest, cancel: CancellationToken) -> Result<ModelResponse, ModelError> {
        let key = resolve_key(&self.secrets, "openai", &self.key_name).await?;
        let mut replay = !self.replay_refused.load(Ordering::Relaxed);
        loop {
            let (body, names) = build_request(&request, &self.options, replay);
            let headers = [("authorization", Header::Bearer(&key))];
            let response = match self.http.post_json(&self.url, &headers, &body, true, &cancel).await {
                Ok(response) => response,
                Err(error) => {
                    // The API did not accept the reasoning items we sent back: say so once and send the portable form.
                    let text = error.to_string().to_lowercase();
                    let about_items = text.contains("reasoning")
                        || text.contains("required item")
                        || text.contains("was provided without")
                        || text.contains("items are not persisted");
                    if replay && error.kind() == ModelErrorKind::Invalid && about_items {
                        self.replay_refused.store(true, Ordering::Relaxed);
                        replay = false;
                        continue;
                    }
                    return Err(error);
                }
            };
            let mut reader = self.http.sse(response);
            let mut aggregator = ResponsesAggregator::new();
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

    async fn embed(
        &self,
        request: zorix_core::EmbeddingRequest,
        cancel: CancellationToken,
    ) -> Result<zorix_core::EmbeddingResponse, ModelError> {
        let key = resolve_key(&self.secrets, "openai", &self.key_name).await?;
        let url = crate::embed::sibling_url(&self.url, "responses");
        let batch = crate::embed::lookup("openai", &request.model).map_or(crate::embed::DEFAULT_BATCH, |m| m.max_batch);
        crate::embed::openai_embed(&self.http, &url, Some(&key), &self.name, &request, batch, &cancel).await
    }

    fn embedding_profile(&self, model: &str) -> Option<zorix_core::EmbeddingProfile> {
        crate::embed::lookup("openai", model).map(|m| m.profile())
    }

    fn describe(&self, model: &str) -> Option<ModelProfile> {
        self.catalog.get(&self.name, model).map(|m| m.profile())
    }
}
