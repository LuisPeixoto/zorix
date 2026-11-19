//! The adapter for Ollama's native `/api/chat`, with model discovery.
//!
//! Ollama's OpenAI-compatible endpoint cannot set the context window, and a model is often loaded with far less context than
//! it supports; the loop would then be silently truncated. The native API takes `options.num_ctx`, so the adapter fixes it and
//! tells the runtime the same number, which keeps the context budget honest.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;
use reqwest::Url;
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;
use zorix_core::{
    FinishReason, ModelError, ModelErrorKind, ModelGateway, ModelProfile, ModelRequest, ModelResponse, NoSink, Role,
    TextSink, ToolCallId, ToolCallRequest, Usage,
};

use crate::adapter::{Discover, ToolNameMap, schema_instruction, simple_name_char};
use crate::catalog::{Catalog, Latency, ModelInfo};
use crate::http::{HttpClient, shorten};
use crate::openai_chat::strip_think;

/// What can be tuned.
#[derive(Clone, Debug)]
pub struct OllamaOptions {
    /// The most context to ask Ollama to load, however much the model supports. Bigger windows use much more memory.
    pub max_context: u64,
    /// The context to assume for a model that did not say.
    pub default_context: u64,
}

impl Default for OllamaOptions {
    fn default() -> Self {
        Self { max_context: 32_768, default_context: 8_192 }
    }
}

/// An Ollama instance.
pub struct OllamaGateway {
    name: String,
    http: HttpClient,
    base: Url,
    catalog: Arc<Catalog>,
    options: OllamaOptions,
    known: RwLock<HashMap<String, ModelInfo>>,
}

impl OllamaGateway {
    /// A gateway for the instance called `name` at `base_url` (usually `http://127.0.0.1:11434`).
    pub fn new(name: &str, http: HttpClient, base_url: &Url, catalog: Arc<Catalog>, options: OllamaOptions) -> Self {
        Self { name: name.into(), http, base: base_url.clone(), catalog, options, known: RwLock::new(HashMap::new()) }
    }

    fn endpoint(&self, route: &str) -> Url {
        Url::parse(&format!("{}/{route}", self.base.as_str().trim_end_matches('/')))
            .unwrap_or_else(|_| self.base.clone())
    }

    fn info(&self, model: &str) -> Option<ModelInfo> {
        self.known.read().get(model).cloned().or_else(|| self.catalog.get(&self.name, model).cloned())
    }

    /// The context window asked of Ollama for `model`.
    fn num_ctx(&self, model: &str) -> u64 {
        match self.info(model) {
            Some(info) => info.context_window.min(self.options.max_context),
            None => self.options.default_context,
        }
        .max(2_048)
    }

    /// Ask Ollama what is installed and what each model can do. Returns the chat models found and remembers them.
    pub async fn refresh(&self, cancel: &CancellationToken) -> Result<Vec<ModelInfo>, ModelError> {
        let tags = self.http.get_json(&self.endpoint("api/tags"), &[], cancel).await.map_err(|e| self.hint(e))?;
        let mut found = Vec::new();
        for entry in tags["models"].as_array().into_iter().flatten().take(100) {
            let Some(name) = entry["name"].as_str().or(entry["model"].as_str()) else { continue };
            let response =
                self.http.post_json(&self.endpoint("api/show"), &[], &json!({"model": name}), false, cancel).await;
            let show = match response {
                Ok(response) => self.http.read_json(response, cancel).await?,
                Err(error) if error.kind() == ModelErrorKind::Cancelled => return Err(error),
                Err(_) => continue, // one model that cannot be described does not hide the others
            };
            if let Some(info) = parse_show(&self.name, name, &show, &self.options) {
                found.push(info);
            }
        }
        let mut known = self.known.write();
        known.clear();
        known.extend(found.iter().map(|i| (i.id.clone(), i.clone())));
        Ok(found)
    }

    /// The embeddings models installed: name, vector size and longest input, read from `/api/show`.
    pub async fn embedding_models(&self, cancel: &CancellationToken) -> Result<Vec<EmbeddingModelInfo>, ModelError> {
        let tags = self.http.get_json(&self.endpoint("api/tags"), &[], cancel).await.map_err(|e| self.hint(e))?;
        let mut found = Vec::new();
        for entry in tags["models"].as_array().into_iter().flatten().take(100) {
            let Some(name) = entry["name"].as_str().or(entry["model"].as_str()) else { continue };
            let response =
                self.http.post_json(&self.endpoint("api/show"), &[], &json!({"model": name}), false, cancel).await;
            let show = match response {
                Ok(response) => self.http.read_json(response, cancel).await?,
                Err(error) if error.kind() == ModelErrorKind::Cancelled => return Err(error),
                Err(_) => continue,
            };
            if let Some(info) = parse_embedding_show(name, &show) {
                found.push(info);
            }
        }
        Ok(found)
    }

    fn hint(&self, error: ModelError) -> ModelError {
        match error {
            ModelError::Provider { kind, message, retry_after_ms } if message.contains("could not connect") => {
                ModelError::Provider {
                    kind,
                    message: format!("{message} Is Ollama running? Start it with `ollama serve`."),
                    retry_after_ms,
                }
            }
            other => other,
        }
    }
}

/// Read the answer of `/api/show` into a catalogue entry. `None` for models that cannot chat (embedding models).
pub fn parse_show(provider: &str, name: &str, show: &Value, options: &OllamaOptions) -> Option<ModelInfo> {
    let capabilities: Vec<&str> =
        show["capabilities"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if !capabilities.is_empty() && !capabilities.contains(&"completion") {
        return None;
    }
    let context = show["model_info"]
        .as_object()
        .and_then(|info| info.iter().find(|(key, _)| key.ends_with(".context_length")).and_then(|(_, v)| v.as_u64()));
    let assumed = context.is_none();
    // What Ollama will actually load, not what the model could do: the budget must match the request.
    let context_window = context.unwrap_or(options.default_context).min(options.max_context);
    let size = show["details"]["parameter_size"].as_str().unwrap_or("");
    let billions: f64 = size.trim_end_matches(['B', 'b']).parse().unwrap_or(0.0);
    let tier = match billions {
        b if b <= 0.0 => 1,
        b if b < 4.0 => 1,
        b if b < 15.0 => 2,
        b if b < 45.0 => 3,
        _ => 4,
    };
    Some(ModelInfo {
        provider: provider.to_string(),
        id: name.to_string(),
        display_name: name.to_string(),
        context_window,
        max_output: (context_window / 4).clamp(512, 8_192),
        tools: capabilities.contains(&"tools"),
        vision: capabilities.contains(&"vision"),
        thinking: capabilities.contains(&"thinking"),
        latency: Latency::Standard,
        tier,
        price: None,
        assumed,
    })
}

/// Build the request body. Pure, so it can be tested without a server.
pub fn build_request(request: &ModelRequest, num_ctx: u64) -> (Value, ToolNameMap) {
    let names = ToolNameMap::new(&request.tools, simple_name_char);
    let mut system: Vec<String> = Vec::new();
    let mut messages: Vec<Value> = Vec::new();
    let mut call_names: HashMap<ToolCallId, String> = HashMap::new();
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
                let mut item = json!({"role": "assistant", "content": message.content});
                if !message.tool_calls.is_empty() {
                    item["tool_calls"] = Value::Array(
                        message
                            .tool_calls
                            .iter()
                            .map(|call| {
                                let wire = names.wire(&call.tool, simple_name_char);
                                call_names.insert(call.id.clone(), wire.clone());
                                json!({"function": {"name": wire, "arguments": if call.arguments.is_object() { call.arguments.clone() } else { json!({}) }}})
                            })
                            .collect(),
                    );
                } else if message.content.trim().is_empty() {
                    continue;
                }
                messages.push(item);
            }
            Role::Tool => {
                let name = message.tool_call_id.as_ref().and_then(|id| call_names.get(id)).cloned().unwrap_or_default();
                messages.push(json!({"role": "tool", "tool_name": name, "content": message.content}));
            }
        }
    }
    if let Some(schema) = &request.response_schema {
        system.push(schema_instruction(schema));
    }
    if !system.is_empty() {
        messages.insert(0, json!({"role": "system", "content": system.join("\n\n")}));
    }
    let mut options = Map::new();
    options.insert("num_ctx".into(), json!(num_ctx));
    if let Some(max) = request.params.max_output_tokens {
        options.insert("num_predict".into(), json!(max));
    }
    let mut body = Map::new();
    body.insert("model".into(), json!(request.model));
    body.insert("messages".into(), Value::Array(messages));
    body.insert("stream".into(), json!(true));
    body.insert("options".into(), Value::Object(options));
    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|t| json!({"type": "function", "function": {"name": names.wire(&t.id, simple_name_char), "description": t.description, "parameters": t.input_schema}}))
            .collect();
        body.insert("tools".into(), Value::Array(tools));
    }
    (Value::Object(body), names)
}

fn line_error(message: &str) -> ModelError {
    let lower = message.to_lowercase();
    let kind = if lower.contains("not found") {
        ModelErrorKind::NotFound
    } else if lower.contains("context") {
        ModelErrorKind::ContextTooLong
    } else if lower.contains("does not support tools") {
        ModelErrorKind::Invalid
    } else {
        ModelErrorKind::Unavailable
    };
    ModelError::provider(kind, format!("ollama: {}", shorten(&zorix_core::redact_text(message), 240)))
}

#[async_trait]
impl Discover for OllamaGateway {
    async fn discover(&self, cancel: &CancellationToken) -> Result<Vec<ModelInfo>, ModelError> {
        self.refresh(cancel).await
    }

    async fn discover_embeddings(
        &self,
        cancel: &CancellationToken,
    ) -> Result<Vec<crate::embed::EmbeddingChoice>, ModelError> {
        Ok(self
            .embedding_models(cancel)
            .await?
            .into_iter()
            .map(|m| crate::embed::EmbeddingChoice {
                provider: self.name.clone(),
                dimensions: m.dimensions.or(crate::embed::lookup("ollama", &m.id).map(|k| k.dimensions)),
                max_input_tokens: m.max_input_tokens,
                local: true,
                installed: true,
                id: m.id,
            })
            .collect())
    }
}

/// An embeddings model found on an Ollama server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddingModelInfo {
    /// The name, as Ollama lists it (with its tag).
    pub id: String,
    /// The size of its vectors, when the model says.
    pub dimensions: Option<u32>,
    /// The longest input, in tokens, when the model says.
    pub max_input_tokens: Option<u64>,
}

/// Read an `/api/show` answer for an embeddings model. `None` for models that are not embedding models.
pub fn parse_embedding_show(name: &str, show: &Value) -> Option<EmbeddingModelInfo> {
    let capabilities: Vec<&str> =
        show["capabilities"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let by_table = crate::embed::lookup("ollama", name).is_some();
    if !capabilities.contains(&"embedding") && !(capabilities.is_empty() && by_table) {
        return None;
    }
    let info = show["model_info"].as_object();
    let number =
        |suffix: &str| info.and_then(|i| i.iter().find(|(k, _)| k.ends_with(suffix)).and_then(|(_, v)| v.as_u64()));
    Some(EmbeddingModelInfo {
        id: name.to_string(),
        dimensions: number(".embedding_length").map(|n| n as u32),
        max_input_tokens: number(".context_length"),
    })
}

#[async_trait]
impl ModelGateway for OllamaGateway {
    async fn embed(
        &self,
        request: zorix_core::EmbeddingRequest,
        cancel: CancellationToken,
    ) -> Result<zorix_core::EmbeddingResponse, ModelError> {
        let known = crate::embed::lookup("ollama", &request.model);
        let prefix = known.map_or("", |m| m.prefix(request.purpose));
        let inputs = crate::embed::prepare_inputs(&self.name, &request, prefix)?;
        let max = known.map_or(crate::embed::DEFAULT_BATCH, |m| m.max_batch);
        let mut vectors: Vec<Vec<f32>> = Vec::with_capacity(inputs.len());
        let mut used = 0u64;
        for batch in crate::embed::batches(&inputs, max) {
            let mut body = json!({"model": request.model, "input": batch, "truncate": true});
            if let Some(dimensions) = request.dimensions {
                body["dimensions"] = json!(dimensions);
            }
            let response = self
                .http
                .post_json(&self.endpoint("api/embed"), &[], &body, false, &cancel)
                .await
                .map_err(|e| self.hint(e))?;
            let answer = self.http.read_json(response, &cancel).await?;
            if let Some(error) = answer["error"].as_str() {
                return Err(line_error(error));
            }
            vectors.extend(answer["embeddings"].as_array().into_iter().flatten().map(crate::embed::floats));
            used += answer["prompt_eval_count"].as_u64().unwrap_or(0);
        }
        Ok(zorix_core::EmbeddingResponse {
            model: request.model.clone(),
            provider: Some(self.name.clone()),
            vectors: crate::embed::finish_vectors(&self.name, inputs.len(), vectors)?,
            usage: Usage { input_tokens: used, ..Usage::default() },
        })
    }

    fn embedding_profile(&self, model: &str) -> Option<zorix_core::EmbeddingProfile> {
        crate::embed::lookup("ollama", model).map(|m| m.profile())
    }

    async fn generate(&self, request: ModelRequest, cancel: CancellationToken) -> Result<ModelResponse, ModelError> {
        self.generate_live(request, cancel, Arc::new(NoSink)).await
    }

    async fn generate_live(
        &self,
        request: ModelRequest,
        cancel: CancellationToken,
        sink: Arc<dyn TextSink>,
    ) -> Result<ModelResponse, ModelError> {
        let (body, names) = build_request(&request, self.num_ctx(&request.model));
        let response = self
            .http
            .post_json(&self.endpoint("api/chat"), &[], &body, true, &cancel)
            .await
            .map_err(|e| self.hint(e))?;
        let mut reader = self.http.ndjson(response);
        let (mut text, mut calls, mut done) = (String::new(), Vec::<ToolCallRequest>::new(), None::<Value>);
        let mut think = crate::adapter::ThinkFilter::default();
        while let Some(line) = reader.next(&cancel).await? {
            if let Some(error) = line["error"].as_str() {
                return Err(line_error(error));
            }
            let piece = line["message"]["content"].as_str().unwrap_or("");
            text.push_str(piece);
            let visible = think.push(piece);
            if !visible.is_empty() {
                sink.text(&visible);
            }
            for call in line["message"]["tool_calls"].as_array().into_iter().flatten() {
                let arguments = &call["function"]["arguments"];
                let arguments = match arguments {
                    Value::String(raw) => serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.clone())),
                    Value::Null => json!({}),
                    other => other.clone(),
                };
                calls.push(ToolCallRequest {
                    id: ToolCallId(String::new()),
                    provider_id: Some(format!("ollama_{}", calls.len())),
                    tool: names.original(call["function"]["name"].as_str().unwrap_or("")),
                    arguments,
                });
            }
            if line["done"].as_bool() == Some(true) {
                let rest = think.flush();
                if !rest.is_empty() {
                    sink.text(&rest);
                }
                done = Some(line);
                break;
            }
        }
        let done = done.ok_or_else(|| {
            ModelError::provider(ModelErrorKind::Unavailable, "ollama: the response ended before it was complete.")
        })?;
        let reason = done["done_reason"].as_str().unwrap_or("stop");
        if reason == "load" || reason == "unload" {
            return Err(ModelError::provider(
                ModelErrorKind::Unavailable,
                "ollama: the model was loaded but did not answer. Try again.",
            ));
        }
        let finish = match reason {
            "length" => FinishReason::Length,
            _ if !calls.is_empty() => FinishReason::ToolCalls,
            _ => FinishReason::Stop,
        };
        let usage = Usage {
            input_tokens: done["prompt_eval_count"].as_u64().unwrap_or(0),
            output_tokens: done["eval_count"].as_u64().unwrap_or(0),
            model_calls: 1,
            ..Usage::default()
        };
        let model = done["model"].as_str().unwrap_or(&request.model).to_string();
        let mut result = ModelResponse::new(model, strip_think(&text), calls, usage, finish);
        result.provider = Some(self.name.clone());
        Ok(result)
    }

    fn describe(&self, model: &str) -> Option<ModelProfile> {
        self.info(model).map(|info| ModelProfile {
            context_window: self.num_ctx(model),
            max_output: info.max_output.min(self.num_ctx(model) / 2),
            tools: info.tools,
            vision: info.vision,
        })
    }
}
