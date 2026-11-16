//! The Gemini adapter against a mock `generateContent` API.

use std::sync::Arc;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zorix_core::{
    ExecutionId, FinishReason, GenerationParams, Message, ModelErrorKind, ModelGateway, ModelRequest, Secret,
    ToolCallId, ToolCallRequest, ToolDefinition,
};
use zorix_providers::secrets::{MemoryStore, SecretStore};
use zorix_providers::{Catalog, GeminiGateway, GeminiOptions, HttpClient, HttpConfig, validate_base_url};

const KEY: &str = "AIza-test-0123456789abcdefghijklmnopqrstuv";
const ROUTE: &str = "/v1beta/models/gemini-test:streamGenerateContent";

fn sse(chunks: &[Value]) -> String {
    chunks.iter().map(|c| format!("data: {c}\n\n")).collect()
}

fn usage() -> Value {
    json!({"promptTokenCount": 100, "cachedContentTokenCount": 60, "candidatesTokenCount": 10, "thoughtsTokenCount": 25, "totalTokenCount": 135})
}

fn text_stream(text: &str) -> String {
    sse(&[
        json!({"candidates": [{"content": {"role": "model", "parts": [{"text": &text[..text.len() / 2]}]}, "index": 0}], "modelVersion": "gemini-test-001"}),
        json!({"candidates": [{"content": {"role": "model", "parts": [{"text": &text[text.len() / 2..]}]}, "finishReason": "STOP", "index": 0}], "usageMetadata": usage()}),
    ])
}

fn tool_stream() -> String {
    sse(&[
        json!({"candidates": [{"content": {"role": "model", "parts": [{"text": "thinking about it", "thought": true}, {"text": "Let me compute."}]}, "index": 0}]}),
        json!({"candidates": [{"content": {"role": "model", "parts": [
            {"functionCall": {"name": "math.evaluate", "args": {"expression": "17*23"}}, "thoughtSignature": "SIG-1"},
            {"functionCall": {"name": "clock.now", "args": {}}}
        ]}, "finishReason": "STOP", "index": 0}], "usageMetadata": usage()}),
    ])
}

fn gateway(server: &MockServer, store: Arc<dyn SecretStore>) -> GeminiGateway {
    let http = HttpClient::new("gemini", HttpConfig::default()).unwrap();
    GeminiGateway::new(
        "gemini",
        http,
        &validate_base_url(&server.uri()).unwrap(),
        store,
        "gemini",
        Arc::new(Catalog::embedded()),
        GeminiOptions::default(),
    )
}

fn with_key() -> Arc<dyn SecretStore> {
    let store = MemoryStore::new();
    store.set("gemini", &Secret::new(KEY)).unwrap();
    Arc::new(store)
}

fn tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            id: "math.evaluate".into(),
            description: "calc".into(),
            input_schema: json!({"type": "object", "additionalProperties": false, "properties": {"expression": {"type": "string"}}, "required": ["expression"]}),
        },
        ToolDefinition {
            id: "clock.now".into(),
            description: "time".into(),
            input_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
        },
    ]
}

fn request(messages: Vec<Message>) -> ModelRequest {
    ModelRequest {
        model: "gemini-test".into(),
        execution_id: ExecutionId::new(),
        messages,
        tools: tools(),
        ..ModelRequest::default()
    }
}

fn base() -> Vec<Message> {
    vec![Message::system("You are inside ZORIX."), Message::user("What is 17*23? And the time.")]
}

async fn call(g: &GeminiGateway, r: ModelRequest) -> Result<zorix_core::ModelResponse, zorix_core::ModelError> {
    g.generate(r, CancellationToken::new()).await
}

async fn bodies(server: &MockServer) -> Vec<Value> {
    server.received_requests().await.unwrap().iter().map(|r| serde_json::from_slice(&r.body).unwrap()).collect()
}

async fn mount(server: &MockServer, body: String) {
    Mock::given(method("POST"))
        .and(path(ROUTE))
        .and(query_param("alt", "sse"))
        .and(header("x-goog-api-key", KEY))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(server)
        .await;
}

#[tokio::test]
async fn a_text_answer_counts_thinking_as_output_and_cached_input_once() {
    let server = MockServer::start().await;
    mount(&server, text_stream("Hello there, friend.")).await;
    let response = call(&gateway(&server, with_key()), request(base())).await.unwrap();
    assert_eq!(response.text, "Hello there, friend.");
    assert_eq!(
        (response.finish, response.provider.as_deref(), response.model.as_str()),
        (FinishReason::Stop, Some("gemini"), "gemini-test-001")
    );
    let u = response.usage;
    assert_eq!((u.input_tokens, u.cache_read_tokens, u.output_tokens, u.reasoning_tokens), (40, 60, 35, 25));
}

#[tokio::test]
async fn the_request_has_system_instruction_translated_tools_and_no_key_in_the_body() {
    let server = MockServer::start().await;
    mount(&server, text_stream("ok")).await;
    let mut r = request(base());
    r.params = GenerationParams { max_output_tokens: Some(777), effort: None };
    call(&gateway(&server, with_key()), r).await.unwrap();
    let body = &bodies(&server).await[0];
    assert_eq!(body["systemInstruction"], json!({"parts": [{"text": "You are inside ZORIX."}]}));
    assert_eq!(body["contents"], json!([{"role": "user", "parts": [{"text": "What is 17*23? And the time."}]}]));
    assert_eq!(body["generationConfig"], json!({"maxOutputTokens": 777}));
    let declarations = &body["tools"][0]["functionDeclarations"];
    assert_eq!(declarations[0]["name"], "math.evaluate", "dots are allowed in Gemini names");
    assert!(declarations[0]["parameters"].get("additionalProperties").is_none());
    assert_eq!(declarations[0]["parameters"]["required"], json!(["expression"]));
    assert!(declarations[1].get("parameters").is_none(), "a tool without parameters omits the field");
    assert!(!body.to_string().contains(KEY));
}

#[tokio::test]
async fn function_calls_thought_parts_and_signatures_are_handled() {
    let server = MockServer::start().await;
    mount(&server, tool_stream()).await;
    let response = call(&gateway(&server, with_key()), request(base())).await.unwrap();
    assert_eq!(response.finish, FinishReason::ToolCalls);
    assert_eq!(response.text, "Let me compute.", "a thought part is not part of the answer");
    assert_eq!(response.tool_calls.len(), 2);
    assert_eq!(
        (response.tool_calls[0].tool.as_str(), response.tool_calls[0].arguments.clone()),
        ("math.evaluate", json!({"expression": "17*23"}))
    );
    assert_eq!(response.tool_calls[1].provider_id.as_deref(), Some("gemini_1"));
    let native = response.native.unwrap();
    assert_eq!(native.content["parts"][2]["thoughtSignature"], "SIG-1");
}

fn after_tools(response: &zorix_core::ModelResponse) -> Vec<Message> {
    let calls: Vec<ToolCallRequest> = response
        .tool_calls
        .iter()
        .enumerate()
        .map(|(i, c)| ToolCallRequest { id: ToolCallId(format!("call_1_{i}")), ..c.clone() })
        .collect();
    let mut messages = base();
    messages.push(Message::assistant(response.text.clone(), calls).with_native(response.native.clone()));
    messages.push(Message::tool_result(ToolCallId("call_1_0".into()), "{\"result\":391}", false));
    messages.push(Message::tool_result(ToolCallId("call_1_1".into()), "boom", true));
    messages
}

#[tokio::test]
async fn the_model_turn_goes_back_with_its_signature_and_results_share_one_user_turn() {
    let server = MockServer::start().await;
    mount(&server, tool_stream()).await;
    let g = gateway(&server, with_key());
    let first = call(&g, request(base())).await.unwrap();
    server.reset().await;
    mount(&server, text_stream("done")).await;
    call(&g, request(after_tools(&first))).await.unwrap();

    let contents = bodies(&server).await[0]["contents"].as_array().unwrap().clone();
    assert_eq!(contents.len(), 3);
    assert_eq!(contents[1]["role"], "model");
    assert_eq!(contents[1]["parts"][2]["thoughtSignature"], "SIG-1", "the signature came back exactly");
    assert_eq!(contents[1]["parts"].as_array().unwrap().len(), 4, "the thought part is kept too");
    let results = contents[2]["parts"].as_array().unwrap();
    assert_eq!(contents[2]["role"], "user");
    assert_eq!(
        results[0],
        json!({"functionResponse": {"name": "math.evaluate", "response": {"output": "{\"result\":391}"}}})
    );
    assert_eq!(results[1], json!({"functionResponse": {"name": "clock.now", "response": {"error": "boom"}}}));
}

#[tokio::test]
async fn the_portable_form_uses_the_documented_placeholder_signature() {
    let server = MockServer::start().await;
    mount(&server, tool_stream()).await;
    let g = gateway(&server, with_key());
    let first = call(&g, request(base())).await.unwrap();
    server.reset().await;
    mount(&server, text_stream("done")).await;
    let mut edited = request(after_tools(&first));
    edited.history_edited = true;
    call(&g, edited).await.unwrap();
    let parts = &bodies(&server).await[0]["contents"][1]["parts"];
    assert_eq!(parts.as_array().unwrap().len(), 3, "text and two calls, no thought part: {parts}");
    assert_eq!(parts[1]["functionCall"]["name"], "math.evaluate");
    assert_eq!(parts[1]["thoughtSignature"], "skip_thought_signature_validator");
}

#[tokio::test]
async fn a_tool_schema_that_cannot_be_expressed_fails_before_any_request() {
    let server = MockServer::start().await;
    let mut r = request(base());
    r.tools[0].input_schema = json!({"type": "object", "properties": {"x": {"oneOf": [{"type": "string"}]}}});
    let error = call(&gateway(&server, with_key()), r).await.unwrap_err();
    assert_eq!(error.kind(), ModelErrorKind::Invalid);
    assert!(error.to_string().contains("math.evaluate") && error.to_string().contains("oneOf"), "{error}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn blocked_requests_and_unsafe_answers_are_refusals() {
    let server = MockServer::start().await;
    mount(&server, sse(&[json!({"promptFeedback": {"blockReason": "PROHIBITED_CONTENT"}})])).await;
    let g = gateway(&server, with_key());
    assert_eq!(call(&g, request(base())).await.unwrap_err().kind(), ModelErrorKind::Refused);
    server.reset().await;
    mount(
        &server,
        sse(&[json!({"candidates": [{"content": {"role": "model", "parts": []}, "finishReason": "SAFETY"}]})]),
    )
    .await;
    assert_eq!(call(&g, request(base())).await.unwrap_err().kind(), ModelErrorKind::Refused);
    server.reset().await;
    mount(&server, sse(&[json!({"candidates": [{"content": {"role": "model", "parts": [{"text": "cut"}]}, "finishReason": "MAX_TOKENS"}]})])).await;
    assert_eq!(call(&g, request(base())).await.unwrap().finish, FinishReason::Length);
}

#[tokio::test]
async fn failures_are_typed_and_a_bad_key_reported_as_a_400_is_still_an_authentication_problem() {
    let server = MockServer::start().await;
    for (status, body, wanted) in [
        (
            400,
            json!({"error": {"code": 400, "message": "API key not valid. Please pass a valid API key.", "status": "INVALID_ARGUMENT"}}),
            ModelErrorKind::Auth,
        ),
        (
            400,
            json!({"error": {"code": 400, "message": "Request contains an invalid argument.", "status": "INVALID_ARGUMENT"}}),
            ModelErrorKind::Invalid,
        ),
        (
            403,
            json!({"error": {"code": 403, "message": "denied", "status": "PERMISSION_DENIED"}}),
            ModelErrorKind::Auth,
        ),
        (
            404,
            json!({"error": {"code": 404, "message": "models/gemini-test is not found", "status": "NOT_FOUND"}}),
            ModelErrorKind::NotFound,
        ),
        (
            429,
            json!({"error": {"code": 429, "message": "quota", "status": "RESOURCE_EXHAUSTED"}}),
            ModelErrorKind::RateLimited,
        ),
        (
            503,
            json!({"error": {"code": 503, "message": "overloaded", "status": "UNAVAILABLE"}}),
            ModelErrorKind::Unavailable,
        ),
    ] {
        server.reset().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(&server)
            .await;
        let error = call(&gateway(&server, with_key()), request(base())).await.unwrap_err();
        assert_eq!(error.kind(), wanted, "HTTP {status}: {error}");
    }
    server.reset().await;
    mount(&server, sse(&[json!({"candidates": [{"content": {"role": "model", "parts": [{"text": "half"}]}}]})])).await;
    assert!(
        call(&gateway(&server, with_key()), request(base()))
            .await
            .unwrap_err()
            .to_string()
            .contains("ended before it was complete")
    );
}

#[tokio::test]
async fn a_missing_key_never_reaches_the_network() {
    let server = MockServer::start().await;
    let error = call(&gateway(&server, Arc::new(MemoryStore::new())), request(base())).await.unwrap_err();
    assert_eq!(error.kind(), ModelErrorKind::Auth);
    assert!(error.to_string().contains("zorix provider set-key gemini"));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn discovery_lists_the_models_that_can_generate_across_pages() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1beta/models")).and(wiremock::matchers::query_param_is_missing("pageToken")).and(header("x-goog-api-key", KEY)).respond_with(ResponseTemplate::new(200).set_body_json(json!({
        "models": [
            {"name": "models/gemini-test-pro", "displayName": "Gemini Test Pro", "inputTokenLimit": 2000000, "outputTokenLimit": 65536, "supportedGenerationMethods": ["generateContent"]},
            {"name": "models/embedding-x", "supportedGenerationMethods": ["embedContent"]}
        ],
        "nextPageToken": "p2"
    }))).mount(&server).await;
    Mock::given(method("GET")).and(path("/v1beta/models")).and(query_param("pageToken", "p2")).respond_with(ResponseTemplate::new(200).set_body_json(json!({
        "models": [{"name": "models/gemini-test-flash", "inputTokenLimit": 1000000, "outputTokenLimit": 8192, "supportedGenerationMethods": ["generateContent"]}]
    }))).mount(&server).await;
    let found = gateway(&server, with_key()).discover(&CancellationToken::new()).await.unwrap();
    let ids: Vec<&str> = found.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, vec!["gemini-test-pro", "gemini-test-flash"]);
    assert_eq!(found[0].context_window, 2_000_000);
}
