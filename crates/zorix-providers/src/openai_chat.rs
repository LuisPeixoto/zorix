//! The adapter for OpenAI-style Chat Completions servers: OpenRouter, Groq, vLLM, LM Studio and others that copy the API.
//!
//! Chat Completions carries no model-bound reasoning state, so there is no native turn to keep. Servers differ in small ways;
//! the adapter sticks to the common core (messages, tools, streaming, `stream_options.include_usage`).

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use reqwest::Url;
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;
use zorix_core::{
    FinishReason, ModelError, ModelErrorKind, ModelGateway, ModelProfile, ModelRequest, ModelResponse, Role,
    ToolCallId, ToolCallRequest, Usage,
};

use crate::adapter::{ToolNameMap, missing_key, resolve_key, schema_instruction, simple_name_char};
use crate::catalog::Catalog;
use crate::http::{Header, HttpClient, is_loopback};
use crate::secrets::SecretStore;
use crate::sse::SseEvent;

/// What can be tuned.
#[derive(Clone, Debug, Default)]
pub struct OpenAiChatOptions {
    /// Send `max_completion_tokens` (newer OpenAI models) instead of `max_tokens` (everything else).
    pub max_completion_tokens_field: bool,
    /// Insist on an API key. When `None`, a key is required unless the server is on this computer.
    pub require_key: Option<bool>,
}

/// A Chat Completions provider instance.
pub struct OpenAiChatGateway {
    name: String,
    http: HttpClient,
    url: Url,
    secrets: Arc<dyn SecretStore>,
    key_name: String,
    catalog: Arc<Catalog>,
    options: OpenAiChatOptions,
    key_required: bool,
}

impl OpenAiChatGateway {
    /// A gateway for the instance called `name` at `base_url` (such as `https://openrouter.ai/api/v1`).
    pub fn new(
        name: &str,
        http: HttpClient,
        base_url: &Url,
        secrets: Arc<dyn SecretStore>,
        key_name: &str,
        catalog: Arc<Catalog>,
        options: OpenAiChatOptions,
    ) -> Result<Self, String> {
        let url = Url::parse(&format!("{}/chat/completions", base_url.as_str().trim_end_matches('/')))
            .map_err(|e| e.to_string())?;
        let local = base_url.host_str().is_some_and(is_loopback);
        let key_required = options.require_key.unwrap_or(!local);
        Ok(Self { name: name.into(), http, url, secrets, key_name: key_name.into(), catalog, options, key_required })
    }
}

/// Remove `<think>…</think>` blocks that some models write into their answer.
pub fn strip_think(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("<think>") {
        out.push_str(&rest[..start]);
        match rest[start..].find("</think>") {
            Some(end) => rest = &rest[start + end + "</think>".len()..],
            None => return out.trim().to_string(), // an unclosed block: everything after it is thinking
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

/// Build the request body. Pure, so it can be tested without a server.
pub fn build_request(request: &ModelRequest, options: &OpenAiChatOptions) -> (Value, ToolNameMap) {
    let names = ToolNameMap::new(&request.tools, simple_name_char);
    let mut system: Vec<String> = Vec::new();
    let mut messages: Vec<Value> = Vec::new();
    let mut provider_ids: std::collections::HashMap<ToolCallId, String> = std::collections::HashMap::new();
    for message in &request.messages {
        match message.role {
            Role::System => {
                if !message.content.trim().is_empty() {
                    system.push(message.content.clone());
                }
            }
            Role::User => {
                if !message.content.trim().is_empty() {
                    messages.push(json!({"role": "user", "content": message.content}));
                }
            }
            Role::Assistant => {
                for call in &message.tool_calls {
                    provider_ids.insert(call.id.clone(), call.provider_id.clone().unwrap_or_else(|| call.id.0.clone()));
                }
                let mut item = json!({"role": "assistant", "content": message.content});
                if !message.tool_calls.is_empty() {
                    item["tool_calls"] = Value::Array(
                        message
                            .tool_calls
                            .iter()
                            .map(|call| {
                                json!({
                                    "id": provider_ids[&call.id],
                                    "type": "function",
                                    "function": {"name": names.wire(&call.tool, simple_name_char), "arguments": call.arguments.to_string()},
                                })
                            })
                            .collect(),
                    );
                } else if message.content.trim().is_empty() {
                    continue;
                }
                messages.push(item);
            }
            Role::Tool => {
                let id = message.tool_call_id.as_ref();
                let tool_call_id = id
                    .and_then(|id| provider_ids.get(id).cloned())
                    .or_else(|| id.map(|i| i.0.clone()))
                    .unwrap_or_default();
                messages.push(json!({"role": "tool", "tool_call_id": tool_call_id, "content": message.content}));
            }
        }
    }
    if let Some(schema) = &request.response_schema {
        system.push(schema_instruction(schema));
    }
    if !system.is_empty() {
        messages.insert(0, json!({"role": "system", "content": system.join("\n\n")}));
    }

    let mut body = Map::new();
    body.insert("model".into(), json!(request.model));
    body.insert("messages".into(), Value::Array(messages));
    body.insert("stream".into(), json!(true));
    body.insert("stream_options".into(), json!({"include_usage": true}));
    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|t| json!({"type": "function", "function": {"name": names.wire(&t.id, simple_name_char), "description": t.description, "parameters": t.input_schema}}))
            .collect();
        body.insert("tools".into(), Value::Array(tools));
    }
    if let Some(max) = request.params.max_output_tokens {
        let field = if options.max_completion_tokens_field { "max_completion_tokens" } else { "max_tokens" };
        body.insert(field.into(), json!(max));
    }
    (Value::Object(body), names)
}

#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

/// Builds a response out of the stream's chunks.
#[derive(Default)]
pub struct ChatAggregator {
    model: String,
    text: String,
    calls: BTreeMap<u64, PartialCall>,
    finish: Option<String>,
    usage: Option<Value>,
    done: bool,
}

impl ChatAggregator {
    /// A new aggregator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Take one event.
    pub fn on_event(&mut self, event: &SseEvent) -> Result<(), ModelError> {
        let data = event.data.trim();
        if data.is_empty() {
            return Ok(());
        }
        if data == "[DONE]" {
            self.done = true;
            return Ok(());
        }
        let chunk: Value = serde_json::from_str(data).map_err(|_| {
            ModelError::provider(ModelErrorKind::Unavailable, "the stream had an event that was not JSON.")
        })?;
        if !chunk["error"].is_null() {
            let message = chunk["error"]["message"].as_str().or(chunk["error"].as_str()).unwrap_or("an error");
            let code = chunk["error"]["code"].to_string();
            return Err(crate::http::classify_stream_error(
                "server",
                &json!({"type": code.trim_matches('"'), "message": message}),
            ));
        }
        if let Some(model) = chunk["model"].as_str().filter(|m| !m.is_empty()) {
            self.model = model.to_string();
        }
        if !chunk["usage"].is_null() {
            self.usage = Some(chunk["usage"].clone());
        }
        for choice in chunk["choices"].as_array().into_iter().flatten() {
            let delta = &choice["delta"];
            if let Some(text) = delta["content"].as_str() {
                self.text.push_str(text);
            }
            for call in delta["tool_calls"].as_array().into_iter().flatten() {
                let index = call["index"].as_u64().unwrap_or(self.calls.len() as u64);
                let partial = self.calls.entry(index).or_default();
                if let Some(id) = call["id"].as_str().filter(|i| !i.is_empty()) {
                    partial.id = id.to_string();
                }
                if let Some(name) = call["function"]["name"].as_str() {
                    partial.name.push_str(name);
                }
                if let Some(arguments) = call["function"]["arguments"].as_str() {
                    partial.arguments.push_str(arguments);
                }
            }
            if let Some(reason) = choice["finish_reason"].as_str() {
                self.finish = Some(reason.to_string());
            }
        }
        Ok(())
    }

    /// The finished response.
    pub fn finish(self, requested_model: &str, names: &ToolNameMap) -> Result<ModelResponse, ModelError> {
        if self.finish.is_none() && !self.done {
            return Err(ModelError::provider(
                ModelErrorKind::Unavailable,
                "the response ended before it was complete.",
            ));
        }
        if self.finish.as_deref() == Some("content_filter") {
            return Err(ModelError::provider(
                ModelErrorKind::Refused,
                "the provider's content filter declined to answer.",
            ));
        }
        let calls: Vec<ToolCallRequest> = self
            .calls
            .into_iter()
            .filter(|(_, c)| !c.name.is_empty())
            .map(|(index, c)| {
                let arguments = if c.arguments.trim().is_empty() {
                    json!({})
                } else {
                    serde_json::from_str(&c.arguments).unwrap_or_else(|_| Value::String(c.arguments.clone()))
                };
                ToolCallRequest {
                    id: ToolCallId(String::new()),
                    provider_id: Some(if c.id.is_empty() { format!("call_{index}") } else { c.id }),
                    tool: names.original(&c.name),
                    arguments,
                }
            })
            .collect();
        let finish = match self.finish.as_deref() {
            Some("length") => FinishReason::Length,
            _ if !calls.is_empty() => FinishReason::ToolCalls,
            _ => FinishReason::Stop,
        };
        let usage = match &self.usage {
            Some(u) => {
                let cached = u["prompt_tokens_details"]["cached_tokens"].as_u64().unwrap_or(0);
                Usage {
                    input_tokens: u["prompt_tokens"].as_u64().unwrap_or(0).saturating_sub(cached),
                    cache_read_tokens: cached,
                    output_tokens: u["completion_tokens"].as_u64().unwrap_or(0),
                    reasoning_tokens: u["completion_tokens_details"]["reasoning_tokens"].as_u64().unwrap_or(0),
                    model_calls: 1,
                    ..Usage::default()
                }
            }
            None => Usage { model_calls: 1, ..Usage::default() },
        };
        let model = if self.model.is_empty() { requested_model.to_string() } else { self.model };
        Ok(ModelResponse::new(model, strip_think(&self.text), calls, usage, finish))
    }
}

#[async_trait]
impl ModelGateway for OpenAiChatGateway {
    async fn generate(&self, request: ModelRequest, cancel: CancellationToken) -> Result<ModelResponse, ModelError> {
        let key = if self.key_required {
            Some(resolve_key(&self.secrets, &self.name, &self.key_name).await?)
        } else {
            crate::secrets::get_async(self.secrets.clone(), self.key_name.clone()).await.unwrap_or_default()
        };
        if self.key_required && key.is_none() {
            return Err(missing_key(&self.name, &self.key_name));
        }
        let (body, names) = build_request(&request, &self.options);
        let headers: Vec<(&str, Header)> = key.iter().map(|k| ("authorization", Header::Bearer(k))).collect();
        let response = self.http.post_json(&self.url, &headers, &body, true, &cancel).await?;
        let mut reader = self.http.sse(response);
        let mut aggregator = ChatAggregator::new();
        while let Some(event) = reader.next(&cancel).await? {
            aggregator.on_event(&event)?;
        }
        let mut result = aggregator.finish(&request.model, &names)?;
        result.provider = Some(self.name.clone());
        result.usage.cost_micros = self
            .catalog
            .cost_micros(&self.name, &result.model, &result.usage)
            .or_else(|| self.catalog.cost_micros(&self.name, &request.model, &result.usage))
            .unwrap_or(0);
        Ok(result)
    }

    fn describe(&self, model: &str) -> Option<ModelProfile> {
        self.catalog.get(&self.name, model).map(|m| m.profile())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thinking_blocks_written_into_the_answer_are_removed() {
        assert_eq!(strip_think("<think>let me see</think>The answer."), "The answer.");
        assert_eq!(strip_think("a<think>x</think>b<think>y</think>c"), "abc");
        assert_eq!(strip_think("before <think>never closed"), "before");
        assert_eq!(strip_think("no tags"), "no tags");
    }
}
