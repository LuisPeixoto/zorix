//! The Anthropic adapter against a mock server that speaks the Messages API's streaming format.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};
use zorix_core::{
    Effort, ExecutionId, FinishReason, GenerationParams, Message, ModelErrorKind, ModelGateway, ModelRequest,
    NativeTurn, Secret, ToolCallId, ToolCallRequest, ToolDefinition,
};
use zorix_providers::secrets::{MemoryStore, SecretStore};
use zorix_providers::{AnthropicGateway, AnthropicOptions, Catalog, HttpClient, HttpConfig, validate_base_url};

const KEY: &str = "sk-ant-test-0123456789abcdefghijklmnop";

fn sse(events: &[(&str, Value)]) -> String {
    events.iter().map(|(name, data)| format!("event: {name}\ndata: {data}\n\n")).collect()
}

fn stream_response(body: String) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")
}

fn text_stream(model: &str, text: &str) -> String {
    sse(&[
        (
            "message_start",
            json!({"type": "message_start", "message": {"id": "msg_1", "model": model, "usage": {"input_tokens": 25, "output_tokens": 1, "cache_read_input_tokens": 100, "cache_creation_input_tokens": 10}}}),
        ),
        (
            "content_block_start",
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
        ),
        ("ping", json!({"type": "ping"})),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": &text[..text.len() / 2]}}),
        ),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": &text[text.len() / 2..]}}),
        ),
        ("content_block_stop", json!({"type": "content_block_stop", "index": 0})),
        (
            "message_delta",
            json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 15}}),
        ),
        ("message_stop", json!({"type": "message_stop"})),
    ])
}

fn tool_stream(model: &str) -> String {
    sse(&[
        (
            "message_start",
            json!({"type": "message_start", "message": {"id": "msg_2", "model": model, "usage": {"input_tokens": 40, "output_tokens": 1}}}),
        ),
        (
            "content_block_start",
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "thinking", "thinking": ""}}),
        ),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": "I should calculate."}}),
        ),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "signature_delta", "signature": "SIG123"}}),
        ),
        ("content_block_stop", json!({"type": "content_block_stop", "index": 0})),
        (
            "content_block_start",
            json!({"type": "content_block_start", "index": 1, "content_block": {"type": "text", "text": ""}}),
        ),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 1, "delta": {"type": "text_delta", "text": "Let me compute."}}),
        ),
        ("content_block_stop", json!({"type": "content_block_stop", "index": 1})),
        (
            "content_block_start",
            json!({"type": "content_block_start", "index": 2, "content_block": {"type": "tool_use", "id": "toolu_A", "name": "math_evaluate", "input": {}}}),
        ),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 2, "delta": {"type": "input_json_delta", "partial_json": "{\"expres"}}),
        ),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 2, "delta": {"type": "input_json_delta", "partial_json": "sion\": \"17*23\"}"}}),
        ),
        ("content_block_stop", json!({"type": "content_block_stop", "index": 2})),
        (
            "content_block_start",
            json!({"type": "content_block_start", "index": 3, "content_block": {"type": "tool_use", "id": "toolu_B", "name": "clock_now", "input": {}}}),
        ),
        ("content_block_stop", json!({"type": "content_block_stop", "index": 3})),
        (
            "message_delta",
            json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 60}}),
        ),
        ("message_stop", json!({"type": "message_stop"})),
    ])
}

fn gateway(server: &MockServer, store: Arc<dyn SecretStore>) -> AnthropicGateway {
    let http = HttpClient::new("anthropic", HttpConfig::default()).unwrap();
    let base = validate_base_url(&server.uri()).unwrap();
    AnthropicGateway::new(
        "anthropic",
        http,
        &base,
        store,
        "anthropic",
        Arc::new(Catalog::embedded()),
        AnthropicOptions::default(),
    )
    .unwrap()
}

fn with_key() -> Arc<dyn SecretStore> {
    let store = MemoryStore::new();
    store.set("anthropic", &Secret::new(KEY)).unwrap();
    Arc::new(store)
}

fn tools() -> Vec<ToolDefinition> {
    ["math.evaluate", "clock.now"]
        .into_iter()
        .map(|id| ToolDefinition {
            id: id.into(),
            description: format!("the {id} tool"),
            input_schema: json!({"type": "object"}),
        })
        .collect()
}

fn request(messages: Vec<Message>) -> ModelRequest {
    ModelRequest {
        model: "claude-opus-5-5".into(),
        execution_id: ExecutionId::new(),
        messages,
        tools: tools(),
        ..ModelRequest::default()
    }
}

fn base_messages() -> Vec<Message> {
    vec![Message::system("You are inside ZORIX."), Message::user("What is 17*23? Also the time.")]
}

async fn call(
    gateway: &AnthropicGateway,
    request: ModelRequest,
) -> Result<zorix_core::ModelResponse, zorix_core::ModelError> {
    gateway.generate(request, CancellationToken::new()).await
}

async fn bodies(server: &MockServer) -> Vec<Value> {
    server.received_requests().await.unwrap().iter().map(|r| serde_json::from_slice(&r.body).unwrap()).collect()
}

async fn mount_ok(server: &MockServer, body: String) {
    Mock::given(method("POST")).and(path("/v1/messages")).respond_with(stream_response(body)).mount(server).await;
}

#[tokio::test]
async fn a_text_answer_is_assembled_with_usage_cache_and_cost() {
    let server = MockServer::start().await;
    mount_ok(&server, text_stream("claude-opus-5-5", "Hello there, traveller.")).await;
    let response = call(&gateway(&server, with_key()), request(base_messages())).await.unwrap();

    assert_eq!(response.text, "Hello there, traveller.");
    assert_eq!(
        (response.finish, response.provider.as_deref(), response.model.as_str()),
        (FinishReason::Stop, Some("anthropic"), "claude-opus-5-5")
    );
    let usage = response.usage;
    assert_eq!(
        (usage.input_tokens, usage.output_tokens, usage.cache_read_tokens, usage.cache_write_tokens, usage.model_calls),
        (25, 15, 100, 10, 1)
    );
    // 25 in at $4/M + 100 cached at $0.20/M + 10 written at $4/M + 15 out at $20/M = 100 + 20 + 40 + 300 micros.
    assert_eq!(usage.cost_micros, 460);
    assert!(response.native.is_some());
}

#[tokio::test]
async fn the_request_follows_the_rules_of_the_current_api_version() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", KEY))
        .and(header("anthropic-version", "2026-10-21"))
        .respond_with(stream_response(text_stream("claude-opus-5-5", "ok")))
        .mount(&server)
        .await;
    call(&gateway(&server, with_key()), request(base_messages())).await.unwrap();

    let body = &bodies(&server).await[0];
    for forbidden in ["temperature", "top_p", "top_k", "thinking", "tool_choice", "budget_tokens"] {
        assert!(body.get(forbidden).is_none(), "{forbidden} must not be sent");
    }
    assert_eq!(body["model"], "claude-opus-5-5");
    assert_eq!(body["stream"], true);
    assert_eq!(body["max_tokens"], 64_000);
    assert_eq!(body["output_config"], json!({"effort": "high"}));
    assert_eq!(body["cache_control"], json!({"type": "ephemeral"}));
    assert_eq!(body["system"], json!([{"type": "text", "text": "You are inside ZORIX."}]));
    assert_eq!(
        body["messages"],
        json!([{"role": "user", "content": [{"type": "text", "text": "What is 17*23? Also the time."}]}])
    );
    let names: Vec<&str> = body["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["math_evaluate", "clock_now"], "dots are not allowed in tool names");
    assert!(!body.to_string().contains(KEY), "the key is in a header, never in the body");
}

#[tokio::test]
async fn effort_and_output_size_come_from_the_task_and_the_models_own_limits() {
    let server = MockServer::start().await;
    mount_ok(&server, text_stream("claude-opus-5-5", "ok")).await;
    let g = gateway(&server, with_key());
    let mut r = request(base_messages());
    r.params = GenerationParams { max_output_tokens: Some(1_000_000), effort: Some(Effort::Max) };
    call(&g, r).await.unwrap();
    let mut small = request(base_messages());
    small.params = GenerationParams { max_output_tokens: Some(500), effort: Some(Effort::Low) };
    call(&g, small).await.unwrap();
    // A model that does not take effort never gets it.
    let mut old = request(base_messages());
    old.model = "claude-haiku-4-5".into();
    call(&g, old).await.unwrap();

    let all = bodies(&server).await;
    assert_eq!(
        (all[0]["max_tokens"].as_u64(), all[0]["output_config"]["effort"].as_str()),
        (Some(128_000), Some("max")),
        "clamped to the model's limit"
    );
    assert_eq!((all[1]["max_tokens"].as_u64(), all[1]["output_config"]["effort"].as_str()), (Some(500), Some("low")));
    assert!(all[2].get("output_config").is_none());
}

#[tokio::test]
async fn tool_calls_thinking_and_names_are_assembled_and_the_raw_turn_is_kept() {
    let server = MockServer::start().await;
    mount_ok(&server, tool_stream("claude-opus-5-5")).await;
    let response = call(&gateway(&server, with_key()), request(base_messages())).await.unwrap();

    assert_eq!(response.finish, FinishReason::ToolCalls);
    assert_eq!(response.text, "Let me compute.");
    assert_eq!(response.tool_calls.len(), 2);
    assert_eq!(response.tool_calls[0].tool, "math.evaluate", "the wire name is mapped back to the ZORIX id");
    assert_eq!(response.tool_calls[0].arguments, json!({"expression": "17*23"}));
    assert_eq!(response.tool_calls[0].provider_id.as_deref(), Some("toolu_A"));
    assert_eq!(
        (response.tool_calls[1].tool.as_str(), response.tool_calls[1].arguments.clone()),
        ("clock.now", json!({}))
    );

    let native = response.native.unwrap();
    assert_eq!((native.provider.as_str(), native.model.as_str()), ("anthropic", "claude-opus-5-5"));
    let blocks = native.content.as_array().unwrap();
    assert_eq!(blocks[0], json!({"type": "thinking", "thinking": "I should calculate.", "signature": "SIG123"}));
    assert_eq!(blocks[2]["input"], json!({"expression": "17*23"}));
}

/// The conversation after the model asked for two tools: the runtime's view of it.
fn conversation_after_tools(response: &zorix_core::ModelResponse) -> Vec<Message> {
    let calls: Vec<ToolCallRequest> = response
        .tool_calls
        .iter()
        .enumerate()
        .map(|(i, c)| ToolCallRequest { id: ToolCallId(format!("call_1_{i}")), ..c.clone() })
        .collect();
    let mut messages = base_messages();
    messages.push(Message::assistant(response.text.clone(), calls).with_native(response.native.clone()));
    messages.push(Message::tool_result(ToolCallId("call_1_0".into()), "{\"result\":391}", false));
    messages.push(Message::tool_result(ToolCallId("call_1_1".into()), "boom", true));
    messages
}

#[tokio::test]
async fn the_raw_turn_is_sent_back_unchanged_and_parallel_results_share_one_user_message() {
    let server = MockServer::start().await;
    mount_ok(&server, tool_stream("claude-opus-5-5")).await;
    let g = gateway(&server, with_key());
    let first = call(&g, request(base_messages())).await.unwrap();
    server.reset().await;
    mount_ok(&server, text_stream("claude-opus-5-5", "391 and noon")).await;
    call(&g, request(conversation_after_tools(&first))).await.unwrap();

    let sent = &bodies(&server).await[0]["messages"];
    let list = sent.as_array().unwrap();
    assert_eq!(list.len(), 3, "user, assistant, one user message with both results: {sent}");
    assert_eq!(list[1]["role"], "assistant");
    assert_eq!(list[1]["content"][0]["signature"], "SIG123", "the thinking block went back exactly as received");
    assert_eq!(list[1]["content"].as_array().unwrap().len(), 4);
    assert_eq!(list[2]["role"], "user");
    let results = list[2]["content"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!((results[0]["tool_use_id"].as_str(), results[0]["is_error"].is_null()), (Some("toolu_A"), true));
    assert_eq!((results[1]["tool_use_id"].as_str(), results[1]["is_error"].as_bool()), (Some("toolu_B"), Some(true)));
}

#[tokio::test]
async fn when_history_was_edited_or_the_model_changed_the_portable_form_is_sent() {
    let server = MockServer::start().await;
    mount_ok(&server, tool_stream("claude-opus-5-5")).await;
    let g = gateway(&server, with_key());
    let first = call(&g, request(base_messages())).await.unwrap();
    server.reset().await;
    mount_ok(&server, text_stream("claude-sonnet-5-5", "ok")).await;

    let mut edited = request(conversation_after_tools(&first));
    edited.history_edited = true;
    call(&g, edited).await.unwrap();
    let mut other_model = request(conversation_after_tools(&first));
    other_model.model = "claude-sonnet-5-5".into();
    call(&g, other_model).await.unwrap();

    for body in bodies(&server).await {
        let assistant = &body["messages"][1]["content"];
        let types: Vec<&str> = assistant.as_array().unwrap().iter().map(|b| b["type"].as_str().unwrap()).collect();
        assert_eq!(types, vec!["text", "tool_use", "tool_use"], "no thinking block, no signature: {assistant}");
        assert_eq!(assistant[1]["name"], "math_evaluate");
        assert_eq!(assistant[1]["id"], "toolu_A", "tool results still match the provider's ids");
    }
}

#[tokio::test]
async fn a_native_turn_from_another_provider_is_ignored() {
    let server = MockServer::start().await;
    mount_ok(&server, text_stream("claude-opus-5-5", "ok")).await;
    let mut messages = base_messages();
    messages.push(Message::assistant("from elsewhere", vec![]).with_native(Some(NativeTurn {
        provider: "gemini".into(),
        model: "x".into(),
        content: json!([{"weird": true}]),
    })));
    messages.push(Message::user("continue"));
    call(&gateway(&server, with_key()), request(messages)).await.unwrap();
    let body = &bodies(&server).await[0];
    assert_eq!(body["messages"][1]["content"], json!([{"type": "text", "text": "from elsewhere"}]));
}

#[tokio::test]
async fn consecutive_user_messages_are_merged_and_empty_ones_dropped() {
    let server = MockServer::start().await;
    mount_ok(&server, text_stream("claude-opus-5-5", "ok")).await;
    let mut messages = base_messages();
    messages.push(Message::assistant("   ", vec![]));
    messages.push(Message::user("Your answer was rejected: too short. Answer again."));
    call(&gateway(&server, with_key()), request(messages)).await.unwrap();
    let sent = &bodies(&server).await[0]["messages"];
    assert_eq!(sent.as_array().unwrap().len(), 1, "{sent}");
    assert_eq!(sent[0]["content"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn a_structured_answer_is_requested_with_a_plain_instruction() {
    let server = MockServer::start().await;
    mount_ok(&server, text_stream("claude-opus-5-5", "{}")).await;
    let mut r = request(base_messages());
    r.response_schema = Some(json!({"type": "object", "required": ["total"]}));
    call(&gateway(&server, with_key()), r).await.unwrap();
    let system = bodies(&server).await[0]["system"].to_string();
    assert!(system.contains("single JSON value") && system.contains("total"), "{system}");
}

#[tokio::test]
async fn a_refusal_is_a_typed_error_the_router_can_fall_back_on() {
    let server = MockServer::start().await;
    let body = sse(&[
        (
            "message_start",
            json!({"type": "message_start", "message": {"model": "claude-opus-5-5", "usage": {"input_tokens": 5, "output_tokens": 0}}}),
        ),
        (
            "message_delta",
            json!({"type": "message_delta", "delta": {"stop_reason": "refusal", "stop_details": {"type": "refusal", "category": "cyber"}}, "usage": {"output_tokens": 0}}),
        ),
        ("message_stop", json!({"type": "message_stop"})),
    ]);
    mount_ok(&server, body).await;
    let error = call(&gateway(&server, with_key()), request(base_messages())).await.unwrap_err();
    assert_eq!(error.kind(), ModelErrorKind::Refused);
    assert!(error.to_string().contains("cyber"), "{error}");
}

#[tokio::test]
async fn http_failures_are_typed_and_a_missing_key_never_reaches_the_network() {
    let server = MockServer::start().await;
    for (status, wanted) in [
        (401, ModelErrorKind::Auth),
        (404, ModelErrorKind::NotFound),
        (413, ModelErrorKind::ContextTooLong),
        (429, ModelErrorKind::RateLimited),
        (500, ModelErrorKind::Unavailable),
        (529, ModelErrorKind::Unavailable),
    ] {
        server.reset().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status).insert_header("retry-after", "3"))
            .mount(&server)
            .await;
        let error = call(&gateway(&server, with_key()), request(base_messages())).await.unwrap_err();
        assert_eq!(error.kind(), wanted, "HTTP {status}");
    }
    server.reset().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "3"))
        .mount(&server)
        .await;
    match call(&gateway(&server, with_key()), request(base_messages())).await.unwrap_err() {
        zorix_core::ModelError::Provider { retry_after_ms, .. } => assert_eq!(retry_after_ms, Some(3_000)),
        other => panic!("{other:?}"),
    }

    server.reset().await;
    let error = call(&gateway(&server, Arc::new(MemoryStore::new())), request(base_messages())).await.unwrap_err();
    assert_eq!(error.kind(), ModelErrorKind::Auth);
    assert!(error.to_string().contains("zorix provider set-key anthropic"));
    assert!(server.received_requests().await.unwrap().is_empty(), "no request was made without a key");
}

#[tokio::test]
async fn an_error_in_the_middle_of_the_stream_or_a_stream_cut_short_is_an_error() {
    let server = MockServer::start().await;
    let overloaded = sse(&[
        (
            "message_start",
            json!({"type": "message_start", "message": {"model": "m", "usage": {"input_tokens": 1, "output_tokens": 0}}}),
        ),
        ("error", json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}})),
    ]);
    mount_ok(&server, overloaded).await;
    let error = call(&gateway(&server, with_key()), request(base_messages())).await.unwrap_err();
    assert_eq!(error.kind(), ModelErrorKind::Unavailable);
    assert!(error.to_string().contains("overloaded_error"));

    server.reset().await;
    let cut = sse(&[(
        "message_start",
        json!({"type": "message_start", "message": {"model": "m", "usage": {"input_tokens": 1, "output_tokens": 0}}}),
    )]);
    mount_ok(&server, cut).await;
    let error = call(&gateway(&server, with_key()), request(base_messages())).await.unwrap_err();
    assert_eq!(error.kind(), ModelErrorKind::Unavailable);
    assert!(error.to_string().contains("ended before it was complete"));
}

#[tokio::test]
async fn the_server_side_fallback_is_asked_for_and_dropped_for_good_when_the_account_does_not_know_it() {
    let server = MockServer::start().await;
    // First: refuse the beta. Then: answer normally.
    Mock::given(method("POST"))
        .and(header("anthropic-beta", "server-side-fallback-2026-07-01"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": {"type": "invalid_request_error", "message": "Unexpected value(s) `server-side-fallback-2026-07-01` for the `anthropic-beta` header."}})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(stream_response(text_stream("claude-opus-5-5", "fine")))
        .mount(&server)
        .await;
    let g = gateway(&server, with_key());

    let first = call(&g, request(base_messages())).await.unwrap();
    assert_eq!(first.text, "fine");
    let sent = bodies(&server).await;
    assert_eq!(sent[0]["fallbacks"], "default", "the first attempt opted in");
    assert!(sent[1].get("fallbacks").is_none(), "the retry did not");

    call(&g, request(base_messages())).await.unwrap();
    let all = server.received_requests().await.unwrap();
    let headers_of_last: Vec<String> =
        all.last().unwrap().headers.iter().map(|(k, _)| k.as_str().to_string()).collect();
    assert!(!headers_of_last.contains(&"anthropic-beta".to_string()), "remembered: not asked again");
    assert_eq!(all.len(), 3, "one wasted request in total");
}

#[tokio::test]
async fn cancelling_mid_stream_returns_at_once() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(stream_response(text_stream("claude-opus-5-5", "slow")).set_delay(Duration::from_secs(10)))
        .mount(&server)
        .await;
    let cancel = CancellationToken::new();
    let stop = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        stop.cancel();
    });
    let started = std::time::Instant::now();
    let result = gateway(&server, with_key()).generate(request(base_messages()), cancel).await;
    assert!(matches!(result, Err(zorix_core::ModelError::Cancelled)));
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn the_gateway_knows_its_models_windows() {
    let server = MockServer::start().await;
    let g = gateway(&server, with_key());
    let profile = g.describe("claude-opus-5-5").unwrap();
    assert_eq!((profile.context_window, profile.max_output, profile.tools), (1_000_000, 128_000, true));
    assert!(g.describe("claude-unknown-9").is_none());
}

#[tokio::test]
async fn a_tool_call_with_cut_off_arguments_reaches_the_runtime_as_text_so_validation_can_say_why() {
    let server = MockServer::start().await;
    let body = sse(&[
        (
            "message_start",
            json!({"type": "message_start", "message": {"model": "claude-opus-5-5", "usage": {"input_tokens": 1, "output_tokens": 0}}}),
        ),
        (
            "content_block_start",
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": "toolu_X", "name": "math_evaluate", "input": {}}}),
        ),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": "{\"expression\": \"1+"}}),
        ),
        (
            "message_delta",
            json!({"type": "message_delta", "delta": {"stop_reason": "max_tokens"}, "usage": {"output_tokens": 9}}),
        ),
        ("message_stop", json!({"type": "message_stop"})),
    ]);
    mount_ok(&server, body).await;
    let response = call(&gateway(&server, with_key()), request(base_messages())).await.unwrap();
    assert_eq!(response.finish, FinishReason::Length);
    assert!(response.tool_calls[0].arguments.is_string(), "{:?}", response.tool_calls[0].arguments);
}

#[tokio::test]
async fn the_key_is_in_one_header_and_nowhere_else() {
    let server = MockServer::start().await;
    mount_ok(&server, text_stream("claude-opus-5-5", "ok")).await;
    call(&gateway(&server, with_key()), request(base_messages())).await.unwrap();
    let all = server.received_requests().await.unwrap();
    let request: &Request = &all[0];
    let holders: Vec<&str> = request
        .headers
        .iter()
        .filter(|(_, v)| v.to_str().is_ok_and(|v| v.contains(KEY)))
        .map(|(k, _)| k.as_str())
        .collect();
    assert_eq!(holders, vec!["x-api-key"]);
    assert!(!request.url.to_string().contains(KEY));
}
