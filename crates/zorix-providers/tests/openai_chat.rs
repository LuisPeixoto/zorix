//! The Chat Completions adapter against a mock server (the shape OpenRouter, Groq, vLLM and LM Studio share).

use std::sync::Arc;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zorix_core::{
    ExecutionId, FinishReason, GenerationParams, Message, ModelErrorKind, ModelGateway, ModelRequest, Secret,
    ToolCallId, ToolCallRequest, ToolDefinition,
};
use zorix_providers::secrets::{MemoryStore, SecretStore};
use zorix_providers::{Catalog, HttpClient, HttpConfig, OpenAiChatGateway, OpenAiChatOptions, validate_base_url};

const KEY: &str = "sk-or-test-0123456789abcdefghijklmnop";

fn data(chunks: &[Value]) -> String {
    let mut out: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
    out.push_str("data: [DONE]\n\n");
    out
}

fn text_chunks(text: &str) -> String {
    data(&[
        json!({"id": "c1", "model": "llama-test", "choices": [{"index": 0, "delta": {"role": "assistant", "content": ""}, "finish_reason": null}]}),
        json!({"choices": [{"index": 0, "delta": {"content": &text[..text.len() / 2]}, "finish_reason": null}]}),
        json!({"choices": [{"index": 0, "delta": {"content": &text[text.len() / 2..]}, "finish_reason": null}]}),
        json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}),
        json!({"choices": [], "usage": {"prompt_tokens": 50, "completion_tokens": 9, "prompt_tokens_details": {"cached_tokens": 20}, "completion_tokens_details": {"reasoning_tokens": 3}}}),
    ])
}

fn tool_chunks() -> String {
    data(&[
        json!({"model": "llama-test", "choices": [{"index": 0, "delta": {"role": "assistant", "content": null, "tool_calls": [{"index": 0, "id": "call_X", "type": "function", "function": {"name": "math_evaluate", "arguments": ""}}]}, "finish_reason": null}]}),
        json!({"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "function": {"arguments": "{\"expres"}}]}, "finish_reason": null}]}),
        json!({"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "function": {"arguments": "sion\":\"17*23\"}"}}]}, "finish_reason": null}]}),
        json!({"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 1, "function": {"name": "clock_now", "arguments": "{}"}}]}, "finish_reason": null}]}),
        json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]}),
        json!({"choices": [], "usage": {"prompt_tokens": 60, "completion_tokens": 20}}),
    ])
}

fn gateway(server: &MockServer, store: Arc<dyn SecretStore>, options: OpenAiChatOptions) -> OpenAiChatGateway {
    let http = HttpClient::new("openrouter", HttpConfig::default()).unwrap();
    let base = validate_base_url(&format!("{}/api/v1", server.uri())).unwrap();
    OpenAiChatGateway::new("openrouter", http, &base, store, "openrouter", Arc::new(Catalog::embedded()), options)
        .unwrap()
}

fn with_key() -> Arc<dyn SecretStore> {
    let store = MemoryStore::new();
    store.set("openrouter", &Secret::new(KEY)).unwrap();
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
        model: "meta/llama-test".into(),
        execution_id: ExecutionId::new(),
        messages,
        tools: tools(),
        ..ModelRequest::default()
    }
}

fn base() -> Vec<Message> {
    vec![
        Message::system("You are inside ZORIX."),
        Message::system("Be brief."),
        Message::user("What is 17*23? And the time."),
    ]
}

async fn call(g: &OpenAiChatGateway, r: ModelRequest) -> Result<zorix_core::ModelResponse, zorix_core::ModelError> {
    g.generate(r, CancellationToken::new()).await
}

async fn bodies(server: &MockServer) -> Vec<Value> {
    server.received_requests().await.unwrap().iter().map(|r| serde_json::from_slice(&r.body).unwrap()).collect()
}

async fn mount(server: &MockServer, body: String) {
    Mock::given(method("POST"))
        .and(path("/api/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(server)
        .await;
}

#[tokio::test]
async fn a_text_answer_is_assembled_and_thinking_tags_are_removed() {
    let server = MockServer::start().await;
    mount(&server, text_chunks("<think>hmm</think>It is 391.")).await;
    let response = call(&gateway(&server, with_key(), OpenAiChatOptions::default()), request(base())).await.unwrap();
    assert_eq!(response.text, "It is 391.");
    assert_eq!(
        (response.finish, response.provider.as_deref(), response.model.as_str()),
        (FinishReason::Stop, Some("openrouter"), "llama-test")
    );
    let u = response.usage;
    assert_eq!((u.input_tokens, u.cache_read_tokens, u.output_tokens, u.reasoning_tokens), (30, 20, 9, 3));
    assert!(response.native.is_none(), "this API has no reasoning state to keep");
}

#[tokio::test]
async fn the_request_has_one_system_message_streams_usage_and_carries_the_key_in_one_header() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("authorization", format!("Bearer {KEY}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_raw(text_chunks("ok"), "text/event-stream"))
        .mount(&server)
        .await;
    let mut r = request(base());
    r.params = GenerationParams { max_output_tokens: Some(256), effort: None };
    call(&gateway(&server, with_key(), OpenAiChatOptions::default()), r.clone()).await.unwrap();
    call(
        &gateway(
            &server,
            with_key(),
            OpenAiChatOptions { max_completion_tokens_field: true, ..OpenAiChatOptions::default() },
        ),
        r,
    )
    .await
    .unwrap();
    let all = bodies(&server).await;
    let body = &all[0];
    assert_eq!(body["stream_options"], json!({"include_usage": true}));
    assert_eq!(body["messages"][0], json!({"role": "system", "content": "You are inside ZORIX.\n\nBe brief."}));
    assert_eq!(body["messages"].as_array().unwrap().len(), 2);
    assert_eq!(body["tools"][0]["function"]["name"], "math_evaluate");
    assert_eq!((body["max_tokens"].as_u64(), body.get("max_completion_tokens")), (Some(256), None));
    assert_eq!((all[1]["max_completion_tokens"].as_u64(), all[1].get("max_tokens")), (Some(256), None));
    for forbidden in ["temperature", "tool_choice", "top_p"] {
        assert!(body.get(forbidden).is_none());
    }
    assert!(!body.to_string().contains(KEY));
}

#[tokio::test]
async fn streamed_tool_calls_are_stitched_together_by_index_and_names_mapped_back() {
    let server = MockServer::start().await;
    mount(&server, tool_chunks()).await;
    let response = call(&gateway(&server, with_key(), OpenAiChatOptions::default()), request(base())).await.unwrap();
    assert_eq!(response.finish, FinishReason::ToolCalls);
    assert_eq!(response.tool_calls.len(), 2);
    assert_eq!(
        (response.tool_calls[0].tool.as_str(), response.tool_calls[0].arguments.clone()),
        ("math.evaluate", json!({"expression": "17*23"}))
    );
    assert_eq!(response.tool_calls[0].provider_id.as_deref(), Some("call_X"));
    assert_eq!(response.tool_calls[1].tool, "clock.now");
    assert_eq!(
        response.tool_calls[1].provider_id.as_deref(),
        Some("call_1"),
        "a server that sends no id gets a stable one"
    );
}

#[tokio::test]
async fn tool_results_go_back_as_tool_messages_with_the_providers_ids() {
    let server = MockServer::start().await;
    mount(&server, tool_chunks()).await;
    let g = gateway(&server, with_key(), OpenAiChatOptions::default());
    let first = call(&g, request(base())).await.unwrap();
    server.reset().await;
    mount(&server, text_chunks("done")).await;

    let calls: Vec<ToolCallRequest> = first
        .tool_calls
        .iter()
        .enumerate()
        .map(|(i, c)| ToolCallRequest { id: ToolCallId(format!("call_1_{i}")), ..c.clone() })
        .collect();
    let mut messages = base();
    messages.push(Message::assistant("", calls));
    messages.push(Message::tool_result(ToolCallId("call_1_0".into()), "{\"result\":391}", false));
    messages.push(Message::tool_result(ToolCallId("call_1_1".into()), "noon", false));
    call(&g, request(messages)).await.unwrap();

    let sent = bodies(&server).await[0]["messages"].as_array().unwrap().clone();
    let roles: Vec<&str> = sent.iter().map(|m| m["role"].as_str().unwrap()).collect();
    assert_eq!(roles, vec!["system", "user", "assistant", "tool", "tool"]);
    assert_eq!(sent[2]["tool_calls"][0]["id"], "call_X");
    assert_eq!(sent[2]["tool_calls"][0]["function"]["name"], "math_evaluate");
    assert_eq!(sent[2]["tool_calls"][0]["function"]["arguments"], "{\"expression\":\"17*23\"}");
    assert_eq!((sent[3]["tool_call_id"].as_str(), sent[4]["tool_call_id"].as_str()), (Some("call_X"), Some("call_1")));
}

#[tokio::test]
async fn the_key_is_optional_for_a_server_on_this_computer_and_required_elsewhere() {
    let server = MockServer::start().await; // 127.0.0.1: local
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(text_chunks("hi"), "text/event-stream"))
        .mount(&server)
        .await;
    let local = gateway(&server, Arc::new(MemoryStore::new()), OpenAiChatOptions::default());
    call(&local, request(base())).await.unwrap();
    assert!(!server.received_requests().await.unwrap()[0].headers.contains_key("authorization"), "no key, no header");

    let strict = gateway(
        &server,
        Arc::new(MemoryStore::new()),
        OpenAiChatOptions { require_key: Some(true), ..OpenAiChatOptions::default() },
    );
    let error = call(&strict, request(base())).await.unwrap_err();
    assert_eq!(error.kind(), ModelErrorKind::Auth);
    assert!(error.to_string().contains("zorix provider set-key openrouter"));
    assert_eq!(server.received_requests().await.unwrap().len(), 1, "the strict one never sent a request");

    // With a key stored, even a local server gets it.
    call(&gateway(&server, with_key(), OpenAiChatOptions::default()), request(base())).await.unwrap();
    assert!(server.received_requests().await.unwrap()[1].headers.get("authorization").is_some());
}

#[tokio::test]
async fn finishing_reasons_and_failures_are_typed() {
    let server = MockServer::start().await;
    mount(&server, data(&[json!({"choices": [{"index": 0, "delta": {"content": "cut"}, "finish_reason": "length"}]})]))
        .await;
    let g = gateway(&server, with_key(), OpenAiChatOptions::default());
    let response = call(&g, request(base())).await.unwrap();
    assert_eq!((response.finish, response.text.as_str()), (FinishReason::Length, "cut"));

    server.reset().await;
    mount(&server, data(&[json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "content_filter"}]})])).await;
    assert_eq!(call(&g, request(base())).await.unwrap_err().kind(), ModelErrorKind::Refused);

    server.reset().await;
    mount(&server, data(&[json!({"error": {"message": "Rate limit reached", "code": "rate_limit_exceeded"}})])).await;
    assert_eq!(call(&g, request(base())).await.unwrap_err().kind(), ModelErrorKind::RateLimited);

    server.reset().await;
    mount(
        &server,
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"half\"},\"finish_reason\":null}]}\n\n".to_string(),
    )
    .await;
    let error = call(&g, request(base())).await.unwrap_err();
    assert!(error.to_string().contains("ended before it was complete"), "{error}");

    for (status, wanted) in [
        (401, ModelErrorKind::Auth),
        (404, ModelErrorKind::NotFound),
        (429, ModelErrorKind::RateLimited),
        (502, ModelErrorKind::Unavailable),
    ] {
        server.reset().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(status)).mount(&server).await;
        assert_eq!(call(&g, request(base())).await.unwrap_err().kind(), wanted, "HTTP {status}");
    }
}
