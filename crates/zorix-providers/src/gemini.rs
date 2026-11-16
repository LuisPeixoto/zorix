//! The adapter for Google's Gemini `generateContent` API.
//!
//! Gemini 3 and newer require the `thoughtSignature` of a function-call turn to be sent back, so the raw model turn is kept
//! and replayed unchanged (ADR-0012). Tool parameters use an OpenAPI subset: schemas are translated, and a schema that
//! cannot be expressed is refused rather than sent as something different.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use reqwest::Url;
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;
use zorix_core::{
    FinishReason, ModelError, ModelErrorKind, ModelGateway, ModelProfile, ModelRequest, ModelResponse, NativeTurn,
    Role, ToolCallId, ToolCallRequest, Usage,
};

use crate::adapter::{Discover, ToolNameMap, resolve_key, schema_instruction};
use crate::catalog::{Catalog, Latency, ModelInfo};
use crate::http::{Header, HttpClient};
use crate::secrets::SecretStore;
use crate::sse::SseEvent;

/// The placeholder Google documents for a function call whose signature was not kept. A last resort.
const SKIP_SIGNATURE: &str = "skip_thought_signature_validator";

/// What can be tuned.
#[derive(Clone, Debug)]
pub struct GeminiOptions {
    /// Mark function calls of a portable (non-native) history with the documented placeholder signature, so a run can
    /// continue after the model changed or history was cut.
    pub placeholder_signatures: bool,
}

impl Default for GeminiOptions {
    fn default() -> Self {
        Self { placeholder_signatures: true }
    }
}

/// A Gemini provider instance.
pub struct GeminiGateway {
    name: String,
    http: HttpClient,
    base: Url,
    secrets: Arc<dyn SecretStore>,
    key_name: String,
    catalog: Arc<Catalog>,
    options: GeminiOptions,
}

impl GeminiGateway {
    /// A gateway for the instance called `name` at `base_url` (such as `https://generativelanguage.googleapis.com`).
    pub fn new(
        name: &str,
        http: HttpClient,
        base_url: &Url,
        secrets: Arc<dyn SecretStore>,
        key_name: &str,
        catalog: Arc<Catalog>,
        options: GeminiOptions,
    ) -> Self {
        Self { name: name.into(), http, base: base_url.clone(), secrets, key_name: key_name.into(), catalog, options }
    }

    fn endpoint(&self, route: &str) -> Result<Url, ModelError> {
        Url::parse(&format!("{}/v1beta/{route}", self.base.as_str().trim_end_matches('/'))).map_err(|e| {
            ModelError::provider(ModelErrorKind::Invalid, format!("gemini: the address is not valid: {e}"))
        })
    }

    /// Ask Google which models exist and what their limits are.
    pub async fn discover(&self, cancel: &CancellationToken) -> Result<Vec<ModelInfo>, ModelError> {
        let key = resolve_key(&self.secrets, "gemini", &self.key_name).await?;
        let mut found = Vec::new();
        let mut page = None::<String>;
        for _ in 0..10 {
            let mut url = self.endpoint("models")?;
            url.query_pairs_mut().append_pair("pageSize", "100");
            if let Some(token) = &page {
                url.query_pairs_mut().append_pair("pageToken", token);
            }
            let list = self
                .http
                .get_json(&url, &[("x-goog-api-key", Header::Secret(&key))], cancel)
                .await
                .map_err(map_error)?;
            for model in list["models"].as_array().into_iter().flatten() {
                if let Some(info) = parse_model(&self.name, model) {
                    found.push(info);
                }
            }
            match list["nextPageToken"].as_str() {
                Some(token) if !token.is_empty() => page = Some(token.to_string()),
                _ => break,
            }
        }
        Ok(found)
    }
}

/// Read one entry of `models.list` into a catalogue entry. `None` for models that cannot generate content.
pub fn parse_model(provider: &str, model: &Value) -> Option<ModelInfo> {
    let methods: Vec<&str> =
        model["supportedGenerationMethods"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if !methods.contains(&"generateContent") {
        return None;
    }
    let id = model["name"].as_str()?.trim_start_matches("models/").to_string();
    let context = model["inputTokenLimit"].as_u64().filter(|n| *n > 0);
    let output = model["outputTokenLimit"].as_u64().filter(|n| *n > 0);
    let context_window = context.unwrap_or(128_000);
    Some(ModelInfo {
        provider: provider.to_string(),
        display_name: model["displayName"].as_str().unwrap_or(&id).to_string(),
        context_window,
        max_output: output.unwrap_or(8_192).min(context_window),
        tools: true,
        vision: true,
        thinking: model["thinking"].as_bool().unwrap_or(false),
        latency: if id.contains("flash") { Latency::Fast } else { Latency::Standard },
        tier: if id.contains("flash") { 2 } else { 3 },
        price: None,
        assumed: context.is_none(),
        id,
    })
}

/// Translate a JSON Schema into the OpenAPI subset Gemini accepts.
pub fn to_gemini_schema(schema: &Value) -> Result<Value, String> {
    const DROPPED: &[&str] = &[
        "$schema",
        "$id",
        "additionalProperties",
        "title",
        "default",
        "examples",
        "$comment",
        "readOnly",
        "writeOnly",
        "deprecated",
    ];
    const REFUSED: &[&str] =
        &["$ref", "oneOf", "allOf", "not", "patternProperties", "if", "then", "else", "$defs", "definitions"];
    let Some(object) = schema.as_object() else { return Ok(schema.clone()) };
    let mut out = Map::new();
    for (key, value) in object {
        if DROPPED.contains(&key.as_str()) {
            continue;
        }
        if REFUSED.contains(&key.as_str()) {
            return Err(format!("it uses “{key}”, which Gemini's schema format cannot express"));
        }
        let translated = match key.as_str() {
            "properties" => {
                let mut props = Map::new();
                for (name, inner) in value.as_object().into_iter().flatten() {
                    props.insert(name.clone(), to_gemini_schema(inner).map_err(|e| format!("property “{name}”: {e}"))?);
                }
                Value::Object(props)
            }
            "items" => to_gemini_schema(value)?,
            "anyOf" => {
                Value::Array(value.as_array().into_iter().flatten().map(to_gemini_schema).collect::<Result<_, _>>()?)
            }
            "type" => match value {
                // ["string", "null"] becomes a nullable string.
                Value::Array(kinds) => {
                    let non_null: Vec<&Value> = kinds.iter().filter(|k| *k != "null").collect();
                    if non_null.len() != 1 {
                        return Err("it allows several types at once".into());
                    }
                    if kinds.len() != non_null.len() {
                        out.insert("nullable".into(), json!(true));
                    }
                    non_null[0].clone()
                }
                other => other.clone(),
            },
            _ => value.clone(),
        };
        out.insert(key.clone(), translated);
    }
    Ok(Value::Object(out))
}

fn text_part(text: &str) -> Value {
    json!({"text": text})
}

/// Build the request body. Pure, so it can be tested without a server.
pub fn build_request(
    request: &ModelRequest,
    options: &GeminiOptions,
    max_output: Option<u64>,
) -> Result<(Value, ToolNameMap), ModelError> {
    // Gemini allows `.`, `:` and `-` in names besides letters, digits and `_`.
    let allowed: fn(char) -> bool = |c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-');
    let names = ToolNameMap::new(&request.tools, allowed);
    let replay_native = !request.history_edited;

    let mut system: Vec<String> = Vec::new();
    let mut contents: Vec<(&'static str, Vec<Value>)> = Vec::new();
    let push = |role: &'static str, parts: Vec<Value>, contents: &mut Vec<(&'static str, Vec<Value>)>| {
        if parts.is_empty() {
            return;
        }
        match contents.last_mut() {
            Some((last, existing)) if *last == role => existing.extend(parts),
            _ => contents.push((role, parts)),
        }
    };
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
                    push("user", vec![text_part(&message.content)], &mut contents);
                }
            }
            Role::Assistant => {
                for call in &message.tool_calls {
                    call_names.insert(call.id.clone(), names.wire(&call.tool, allowed));
                }
                let native = message
                    .native
                    .as_ref()
                    .filter(|n| replay_native && n.provider == "gemini" && n.model == request.model);
                let parts = match native.and_then(|n| n.content["parts"].as_array()) {
                    Some(parts) => parts.clone(),
                    None => {
                        let mut parts = Vec::new();
                        if !message.content.trim().is_empty() {
                            parts.push(text_part(&message.content));
                        }
                        for call in &message.tool_calls {
                            let mut part = json!({"functionCall": {"name": names.wire(&call.tool, allowed), "args": if call.arguments.is_object() { call.arguments.clone() } else { json!({}) }}});
                            if options.placeholder_signatures {
                                part["thoughtSignature"] = json!(SKIP_SIGNATURE);
                            }
                            parts.push(part);
                        }
                        parts
                    }
                };
                push("model", parts, &mut contents);
            }
            Role::Tool => {
                let name = message.tool_call_id.as_ref().and_then(|id| call_names.get(id)).cloned().unwrap_or_default();
                let response = if message.is_error {
                    json!({"error": message.content})
                } else {
                    json!({"output": message.content})
                };
                push("user", vec![json!({"functionResponse": {"name": name, "response": response}})], &mut contents);
            }
        }
    }
    if let Some(schema) = &request.response_schema {
        system.push(schema_instruction(schema));
    }

    let mut body = Map::new();
    body.insert(
        "contents".into(),
        Value::Array(contents.into_iter().map(|(role, parts)| json!({"role": role, "parts": parts})).collect()),
    );
    if !system.is_empty() {
        body.insert(
            "systemInstruction".into(),
            json!({"parts": system.iter().map(|s| text_part(s)).collect::<Vec<_>>()}),
        );
    }
    if !request.tools.is_empty() {
        let mut declarations = Vec::new();
        for tool in &request.tools {
            let parameters = to_gemini_schema(&tool.input_schema).map_err(|why| {
                ModelError::provider(
                    ModelErrorKind::Invalid,
                    format!("gemini: the tool “{}” cannot be sent: {why}.", tool.id),
                )
            })?;
            let mut declaration = json!({"name": names.wire(&tool.id, allowed), "description": tool.description});
            // A tool with no parameters must omit the field: an empty object schema is refused.
            if parameters.get("properties").and_then(Value::as_object).is_some_and(|p| !p.is_empty()) {
                declaration["parameters"] = parameters;
            }
            declarations.push(declaration);
        }
        body.insert("tools".into(), json!([{"functionDeclarations": declarations}]));
    }
    let mut generation = Map::new();
    let ceiling = max_output.unwrap_or(u64::MAX);
    if let Some(max) = request.params.max_output_tokens {
        generation.insert("maxOutputTokens".into(), json!(u64::from(max).min(ceiling)));
    }
    if !generation.is_empty() {
        body.insert("generationConfig".into(), Value::Object(generation));
    }
    Ok((Value::Object(body), names))
}

/// Builds a response out of the stream's chunks.
#[derive(Default)]
pub struct GeminiAggregator {
    model: String,
    parts: Vec<Value>,
    finish: Option<String>,
    usage: Option<Value>,
    blocked: Option<String>,
}

impl GeminiAggregator {
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
        let chunk: Value = serde_json::from_str(data).map_err(|_| {
            ModelError::provider(ModelErrorKind::Unavailable, "gemini: an event of the stream was not JSON.")
        })?;
        if !chunk["error"].is_null() {
            let status = reqwest::StatusCode::from_u16(chunk["error"]["code"].as_u64().unwrap_or(500) as u16)
                .unwrap_or(reqwest::StatusCode::INTERNAL_SERVER_ERROR);
            return Err(map_error(crate::http::classify("gemini", status, &chunk.to_string(), None)));
        }
        if let Some(model) = chunk["modelVersion"].as_str() {
            self.model = model.to_string();
        }
        if !chunk["usageMetadata"].is_null() {
            self.usage = Some(chunk["usageMetadata"].clone());
        }
        if let Some(reason) = chunk["promptFeedback"]["blockReason"].as_str() {
            self.blocked = Some(reason.to_string());
        }
        if let Some(candidate) = chunk["candidates"].get(0) {
            for part in candidate["content"]["parts"].as_array().into_iter().flatten() {
                self.parts.push(part.clone());
            }
            if let Some(reason) = candidate["finishReason"].as_str() {
                self.finish = Some(reason.to_string());
            }
        }
        Ok(())
    }

    /// The finished response.
    pub fn finish(self, requested_model: &str, names: &ToolNameMap) -> Result<ModelResponse, ModelError> {
        if let Some(reason) = self.blocked {
            return Err(ModelError::provider(
                ModelErrorKind::Refused,
                format!("gemini: the request was blocked ({reason})."),
            ));
        }
        let Some(reason) = self.finish else {
            return Err(ModelError::provider(
                ModelErrorKind::Unavailable,
                "gemini: the response ended before it was complete.",
            ));
        };
        match reason.as_str() {
            "SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII" | "IMAGE_SAFETY" => {
                return Err(ModelError::provider(
                    ModelErrorKind::Refused,
                    format!("gemini: the model declined to answer ({reason})."),
                ));
            }
            "MALFORMED_FUNCTION_CALL" => {
                return Err(ModelError::provider(
                    ModelErrorKind::Unavailable,
                    "gemini: the model wrote a function call it could not complete. Try again.",
                ));
            }
            _ => {}
        }
        let mut text = String::new();
        let mut calls: Vec<ToolCallRequest> = Vec::new();
        for part in &self.parts {
            if part["thought"].as_bool() == Some(true) {
                continue;
            }
            if let Some(t) = part["text"].as_str() {
                text.push_str(t);
            }
            if part["functionCall"].is_object() {
                let name = part["functionCall"]["name"].as_str().unwrap_or("");
                let args = &part["functionCall"]["args"];
                calls.push(ToolCallRequest {
                    id: ToolCallId(String::new()),
                    provider_id: Some(format!("gemini_{}", calls.len())),
                    tool: names.original(name),
                    arguments: if args.is_null() { json!({}) } else { args.clone() },
                });
            }
        }
        let finish = match reason.as_str() {
            "MAX_TOKENS" => FinishReason::Length,
            _ if !calls.is_empty() => FinishReason::ToolCalls,
            _ => FinishReason::Stop,
        };
        let usage = match &self.usage {
            Some(u) => {
                let cached = u["cachedContentTokenCount"].as_u64().unwrap_or(0);
                let thoughts = u["thoughtsTokenCount"].as_u64().unwrap_or(0);
                Usage {
                    input_tokens: u["promptTokenCount"].as_u64().unwrap_or(0).saturating_sub(cached),
                    cache_read_tokens: cached,
                    output_tokens: u["candidatesTokenCount"].as_u64().unwrap_or(0) + thoughts,
                    reasoning_tokens: thoughts,
                    model_calls: 1,
                    ..Usage::default()
                }
            }
            None => Usage { model_calls: 1, ..Usage::default() },
        };
        let model = if self.model.is_empty() { requested_model.to_string() } else { self.model };
        let mut response = ModelResponse::new(model.clone(), text, calls, usage, finish);
        response.provider = Some("gemini".into());
        // Keep the model's own turn, with its signatures, for the next call.
        response.native = Some(NativeTurn {
            provider: "gemini".into(),
            model: requested_model.to_string(),
            content: json!({"role": "model", "parts": self.parts}),
        });
        Ok(response)
    }
}

/// Google reports a bad key as a 400; the router must see it as an authentication problem.
fn map_error(error: ModelError) -> ModelError {
    match error {
        ModelError::Provider { kind: ModelErrorKind::Invalid, message, retry_after_ms }
            if message.to_lowercase().contains("api key not valid")
                || message.to_lowercase().contains("api_key_invalid") =>
        {
            ModelError::Provider { kind: ModelErrorKind::Auth, message, retry_after_ms }
        }
        other => other,
    }
}

#[async_trait]
impl Discover for GeminiGateway {
    async fn discover(&self, cancel: &CancellationToken) -> Result<Vec<ModelInfo>, ModelError> {
        GeminiGateway::discover(self, cancel).await
    }
}

#[async_trait]
impl ModelGateway for GeminiGateway {
    async fn generate(&self, request: ModelRequest, cancel: CancellationToken) -> Result<ModelResponse, ModelError> {
        let key = resolve_key(&self.secrets, "gemini", &self.key_name).await?;
        let max_output = self.catalog.get(&self.name, &request.model).map(|m| m.max_output);
        let (body, names) = build_request(&request, &self.options, max_output)?;
        let model = request.model.trim_start_matches("models/");
        let mut url = self.endpoint(&format!("models/{model}:streamGenerateContent"))?;
        url.query_pairs_mut().append_pair("alt", "sse");
        let headers = [("x-goog-api-key", Header::Secret(&key))];
        let response = self.http.post_json(&url, &headers, &body, true, &cancel).await.map_err(map_error)?;
        let mut reader = self.http.sse(response);
        let mut aggregator = GeminiAggregator::new();
        while let Some(event) = reader.next(&cancel).await? {
            aggregator.on_event(&event)?;
        }
        let mut result = aggregator.finish(&request.model, &names)?;
        result.usage.cost_micros = self.catalog.cost_micros(&self.name, &request.model, &result.usage).unwrap_or(0);
        Ok(result)
    }

    fn describe(&self, model: &str) -> Option<ModelProfile> {
        self.catalog.get(&self.name, model.trim_start_matches("models/")).map(|m| m.profile())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_lose_what_gemini_does_not_know_and_keep_the_meaning() {
        let schema = json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "path": {"type": "string", "maxLength": 512, "description": "relative"},
                "limit": {"type": ["integer", "null"], "minimum": 1},
                "tags": {"type": "array", "items": {"type": "string", "title": "tag"}}
            },
            "required": ["path"]
        });
        let out = to_gemini_schema(&schema).unwrap();
        assert!(out.get("$schema").is_none() && out.get("additionalProperties").is_none());
        assert_eq!(out["properties"]["limit"], json!({"type": "integer", "minimum": 1, "nullable": true}));
        assert_eq!(out["properties"]["tags"]["items"], json!({"type": "string"}));
        assert_eq!(out["required"], json!(["path"]));
        assert_eq!(out["properties"]["path"]["maxLength"], 512);
    }

    #[test]
    fn what_cannot_be_expressed_is_refused_not_changed() {
        for schema in [
            json!({"oneOf": [{"type": "string"}, {"type": "integer"}]}),
            json!({"type": "object", "properties": {"x": {"$ref": "#/$defs/x"}}}),
            json!({"type": ["string", "integer"]}),
            json!({"allOf": []}),
        ] {
            assert!(to_gemini_schema(&schema).is_err(), "{schema}");
        }
        let error = to_gemini_schema(&json!({"type": "object", "properties": {"x": {"not": {}}}})).unwrap_err();
        assert!(error.contains("property “x”"), "{error}");
    }

    #[test]
    fn models_are_read_from_the_list_and_models_that_cannot_generate_are_skipped() {
        let chat = json!({"name": "models/gemini-test-flash", "displayName": "Gemini Test Flash", "inputTokenLimit": 1048576, "outputTokenLimit": 65536, "supportedGenerationMethods": ["generateContent", "countTokens"]});
        let info = parse_model("gemini", &chat).unwrap();
        assert_eq!(
            (info.id.as_str(), info.context_window, info.max_output, info.latency, info.assumed),
            ("gemini-test-flash", 1_048_576, 65_536, Latency::Fast, false)
        );
        let embed = json!({"name": "models/embed-x", "supportedGenerationMethods": ["embedContent"]});
        assert!(parse_model("gemini", &embed).is_none());
        let sparse = json!({"name": "models/gemini-test-pro", "supportedGenerationMethods": ["generateContent"]});
        assert!(parse_model("gemini", &sparse).unwrap().assumed, "a missing limit is flagged as assumed");
    }
}
