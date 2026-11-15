//! The OpenAI Responses adapter against a mock server.

use std::sync::Arc;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zorix_core::{
    Effort, ExecutionId, FinishReason, GenerationParams, Message, ModelErrorKind, ModelGateway, ModelRequest, Secret,
    ToolCallId, ToolCallRequest, ToolDefinition,
};
use zorix_providers::secrets::{MemoryStore, SecretStore};
use zorix_providers::{Catalog, HttpClient, HttpConfig, OpenAiOptions, OpenAiResponsesGateway, validate_base_url};

const KEY: &str = "sk-openai-test-0123456789abcdefghijklmnop";

fn sse(events: &[Value]) -> String {
    events.iter().map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap_or("message"))).collect()
}

fn completed(output: Value, usage: Value) -> String {
    sse(&[
        json!({"type": "response.created", "response": {"id": "resp_1"}}),
        json!({"type": "response.output_text.delta", "delta": "ignored delta"}),
        json!({"type": "response.completed", "response": {"id": "resp_1", "model": "gpt-5-test", "status": "completed", "output": output, "usage": usage}}),
    ])
}

fn text_output(text: &str) -> Value {
    json!([{"type": "message", "id": "msg_1", "role": "assistant", "content": [{"type": "output_text", "text": text}]}])
}

fn tool_output() -> Value {
    json!([
        {"type": "reasoning", "id": "rs_1", "summary": [], "encrypted_content": "ENC=="},
        {"type": "function_call", "id": "fc_1", "call_id": "call_A", "name": "math_evaluate", "arguments": "{\"expression\":\"17*23\"}"},
        {"type": "function_call", "id": "fc_2", "call_id": "call_B", "name": "clock_now", "arguments": ""}
    ])
}

fn usage() -> Value {
    json!({"input_tokens": 120, "input_tokens_details": {"cached_tokens": 100}, "output_tokens": 30, "output_tokens_details": {"reasoning_tokens": 12}})
}

fn gateway(server: &MockServer, store: Arc<dyn SecretStore>) -> OpenAiResponsesGateway {
    let http = HttpClient::new("openai", HttpConfig::default()).unwrap();
    let base = validate_base_url(&format!("{}/v1", server.uri())).unwrap();
    OpenAiResponsesGateway::new(
        "openai",
        http,
        &base,
        store,
        "openai",
        Arc::new(Catalog::embedded()),
        OpenAiOptions::default(),
    )
    .unwrap()
}

fn with_key() -> Arc<dyn SecretStore> {
    let store = MemoryStore::new();
    store.set("openai", &Secret::new(KEY)).unwrap();
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

fn request(model: &str, messages: Vec<Message>) -> ModelRequest {
    ModelRequest {
        model: model.into(),
        execution_id: ExecutionId::new(),
        messages,
        tools: tools(),
        ..ModelRequest::default()
    }
}

fn base() -> Vec<Message> {
    vec![Message::system("You are inside ZORIX."), Message::user("What is 17*23? Also the time.")]
}

async fn call(
    g: &OpenAiResponsesGateway,
    r: ModelRequest,
) -> Result<zorix_core::ModelResponse, zorix_core::ModelError> {
    g.generate(r, CancellationToken::new()).await
}

async fn bodies(server: &MockServer) -> Vec<Value> {
    server.received_requests().await.unwrap().iter().map(|r| serde_json::from_slice(&r.body).unwrap()).collect()
}

async fn mount(server: &MockServer, body: String) {
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(server)
        .await;
}

#[tokio::test]
async fn a_text_answer_normalises_usage_so_cached_tokens_are_counted_once() {
    let server = MockServer::start().await;
    mount(&server, completed(text_output("The answer is 391."), usage())).await;
    let response = call(&gateway(&server, with_key()), request("gpt-5-test", base())).await.unwrap();
    assert_eq!(response.text, "The answer is 391.");
    assert_eq!(
        (response.finish, response.provider.as_deref(), response.model.as_str()),
        (FinishReason::Stop, Some("openai"), "gpt-5-test")
    );
    let u = response.usage;
    assert_eq!(
        (u.input_tokens, u.cache_read_tokens, u.output_tokens, u.reasoning_tokens, u.model_calls),
        (20, 100, 30, 12, 1)
    );
    assert_eq!(u.cost_micros, 0, "no price is known for this model, so none is invented");
}

#[tokio::test]
async fn the_request_is_stateless_and_asks_for_reasoning_items_only_for_reasoning_models() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(header("authorization", format!("Bearer {KEY}").as_str()))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(completed(text_output("ok"), usage()), "text/event-stream"),
        )
        .mount(&server)
        .await;
    let g = gateway(&server, with_key());
    call(&g, request("gpt-5-test", base())).await.unwrap();
    call(&g, request("gpt-4o-mini", base())).await.unwrap();
    let all = bodies(&server).await;

    let reasoning = &all[0];
    assert_eq!(reasoning["store"], false);
    assert_eq!(reasoning["stream"], true);
    assert_eq!(reasoning["instructions"], "You are inside ZORIX.");
    assert_eq!(reasoning["reasoning"], json!({"effort": "high"}));
    assert_eq!(reasoning["include"], json!(["reasoning.encrypted_content"]));
    assert_eq!(
        reasoning["input"],
        json!([{"role": "user", "content": [{"type": "input_text", "text": "What is 17*23? Also the time."}]}])
    );
    for forbidden in ["temperature", "top_p", "tool_choice", "previous_response_id"] {
        assert!(reasoning.get(forbidden).is_none(), "{forbidden}");
    }
    let tool = &reasoning["tools"][0];
    assert_eq!(
        (tool["type"].as_str(), tool["name"].as_str(), tool["strict"].as_bool()),
        (Some("function"), Some("math_evaluate"), Some(false))
    );
    assert!(!reasoning.to_string().contains(KEY));

    let plain = &all[1];
    assert!(
        plain.get("reasoning").is_none() && plain.get("include").is_none(),
        "a model that does not reason gets neither"
    );
}

#[tokio::test]
async fn effort_and_output_size_follow_the_task() {
    let server = MockServer::start().await;
    mount(&server, completed(text_output("ok"), usage())).await;
    let mut r = request("gpt-5-test", base());
    r.params = GenerationParams { max_output_tokens: Some(900), effort: Some(Effort::Low) };
    call(&gateway(&server, with_key()), r).await.unwrap();
    let body = &bodies(&server).await[0];
    assert_eq!((body["max_output_tokens"].as_u64(), body["reasoning"]["effort"].as_str()), (Some(900), Some("low")));
}

#[tokio::test]
async fn function_calls_keep_the_providers_call_ids_and_names_are_mapped_back() {
    let server = MockServer::start().await;
    mount(&server, completed(tool_output(), usage())).await;
    let response = call(&gateway(&server, with_key()), request("gpt-5-test", base())).await.unwrap();
    assert_eq!(response.finish, FinishReason::ToolCalls);
    assert_eq!(response.tool_calls[0].tool, "math.evaluate");
    assert_eq!(response.tool_calls[0].arguments, json!({"expression": "17*23"}));
    assert_eq!(response.tool_calls[0].provider_id.as_deref(), Some("call_A"));
    assert_eq!(
        (response.tool_calls[1].tool.as_str(), response.tool_calls[1].arguments.clone()),
        ("clock.now", json!({})),
        "empty arguments mean none"
    );
    let native = response.native.unwrap();
    assert_eq!(native.content[0]["encrypted_content"], "ENC==");
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
    messages.push(Message::tool_result(ToolCallId("call_1_1".into()), "12:00", false));
    messages
}

#[tokio::test]
async fn reasoning_items_go_back_unchanged_with_the_outputs_in_order() {
    let server = MockServer::start().await;
    mount(&server, completed(tool_output(), usage())).await;
    let g = gateway(&server, with_key());
    let first = call(&g, request("gpt-5-test", base())).await.unwrap();
    server.reset().await;
    mount(&server, completed(text_output("391 at noon"), usage())).await;
    call(&g, request("gpt-5-test", after_tools(&first))).await.unwrap();

    let input = bodies(&server).await[0]["input"].as_array().unwrap().clone();
    let kinds: Vec<String> =
        input.iter().map(|i| i["type"].as_str().or(i["role"].as_str()).unwrap().to_string()).collect();
    assert_eq!(
        kinds,
        vec!["user", "reasoning", "function_call", "function_call", "function_call_output", "function_call_output"]
    );
    assert_eq!(input[1], tool_output()[0], "the reasoning item is exactly what came back");
    assert_eq!(input[2]["id"], "fc_1");
    assert_eq!((input[4]["call_id"].as_str(), input[5]["call_id"].as_str()), (Some("call_A"), Some("call_B")));
}

#[tokio::test]
async fn the_portable_form_has_no_item_ids_and_no_reasoning() {
    let server = MockServer::start().await;
    mount(&server, completed(tool_output(), usage())).await;
    let g = gateway(&server, with_key());
    let first = call(&g, request("gpt-5-test", base())).await.unwrap();
    server.reset().await;
    mount(&server, completed(text_output("ok"), usage())).await;
    let mut edited = request("gpt-5-test", after_tools(&first));
    edited.history_edited = true;
    call(&g, edited).await.unwrap();
    call(&g, request("gpt-other", after_tools(&first))).await.unwrap();

    for body in bodies(&server).await {
        let input = body["input"].as_array().unwrap();
        assert!(input.iter().all(|i| i["type"] != "reasoning" && i.get("id").is_none()), "{input:?}");
        let call_item = input.iter().find(|i| i["type"] == "function_call").unwrap();
        assert_eq!(
            (call_item["call_id"].as_str(), call_item["name"].as_str()),
            (Some("call_A"), Some("math_evaluate"))
        );
        assert_eq!(call_item["arguments"], "{\"expression\":\"17*23\"}", "arguments travel as a JSON string");
    }
}

#[tokio::test]
async fn a_refusal_is_a_typed_error_and_an_incomplete_answer_is_a_length_stop() {
    let server = MockServer::start().await;
    let refusal = json!([{"type": "message", "role": "assistant", "content": [{"type": "refusal", "refusal": "I can't help with that."}]}]);
    mount(&server, completed(refusal, usage())).await;
    let error = call(&gateway(&server, with_key()), request("gpt-5-test", base())).await.unwrap_err();
    assert_eq!(error.kind(), ModelErrorKind::Refused);

    server.reset().await;
    let body = sse(&[
        json!({"type": "response.incomplete", "response": {"model": "gpt-5-test", "status": "incomplete", "incomplete_details": {"reason": "max_output_tokens"}, "output": text_output("half an ans"), "usage": usage()}}),
    ]);
    mount(&server, body).await;
    let response = call(&gateway(&server, with_key()), request("gpt-5-test", base())).await.unwrap();
    assert_eq!((response.finish, response.text.as_str()), (FinishReason::Length, "half an ans"));
}

#[tokio::test]
async fn failures_in_the_stream_and_over_http_are_typed() {
    let server = MockServer::start().await;
    for (code, wanted) in [
        ("rate_limit_exceeded", ModelErrorKind::RateLimited),
        ("context_length_exceeded", ModelErrorKind::ContextTooLong),
        ("invalid_api_key", ModelErrorKind::Auth),
        ("server_error", ModelErrorKind::Unavailable),
    ] {
        server.reset().await;
        mount(&server, sse(&[json!({"type": "response.failed", "response": {"error": {"code": code, "message": format!("it said {code}")}}})])).await;
        let error = call(&gateway(&server, with_key()), request("gpt-5-test", base())).await.unwrap_err();
        assert_eq!(error.kind(), wanted, "{code}: {error}");
    }
    for (status, wanted) in [
        (401, ModelErrorKind::Auth),
        (404, ModelErrorKind::NotFound),
        (429, ModelErrorKind::RateLimited),
        (503, ModelErrorKind::Unavailable),
    ] {
        server.reset().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(status)).mount(&server).await;
        let error = call(&gateway(&server, with_key()), request("gpt-5-test", base())).await.unwrap_err();
        assert_eq!(error.kind(), wanted, "HTTP {status}");
    }
    server.reset().await;
    mount(&server, sse(&[json!({"type": "response.created", "response": {}})])).await;
    let error = call(&gateway(&server, with_key()), request("gpt-5-test", base())).await.unwrap_err();
    assert!(error.to_string().contains("ended before it was complete"));
}

#[tokio::test]
async fn a_missing_key_never_reaches_the_network() {
    let server = MockServer::start().await;
    let error = call(&gateway(&server, Arc::new(MemoryStore::new())), request("gpt-5-test", base())).await.unwrap_err();
    assert_eq!(error.kind(), ModelErrorKind::Auth);
    assert!(error.to_string().contains("zorix provider set-key openai"));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn when_the_api_refuses_the_reasoning_items_the_portable_form_is_used_from_then_on() {
    let server = MockServer::start().await;
    mount(&server, completed(tool_output(), usage())).await;
    let g = gateway(&server, with_key());
    let first = call(&g, request("gpt-5-test", base())).await.unwrap();
    server.reset().await;
    Mock::given(method("POST"))
        .and(wiremock::matchers::body_string_contains("\"type\":\"reasoning\""))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": {"message": "Item 'fc_1' of type 'function_call' was provided without its required 'reasoning' item: 'rs_1'."}})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(completed(text_output("done"), usage()), "text/event-stream"),
        )
        .mount(&server)
        .await;

    let response = call(&g, request("gpt-5-test", after_tools(&first))).await.unwrap();
    assert_eq!(response.text, "done");
    let sent = bodies(&server).await;
    assert!(
        sent[0].to_string().contains("\"type\":\"reasoning\"")
            && !sent[1].to_string().contains("\"type\":\"reasoning\"")
    );
    call(&g, request("gpt-5-test", after_tools(&first))).await.unwrap();
    assert_eq!(server.received_requests().await.unwrap().len(), 3, "the refused form was not tried again");
}
