//! The Ollama adapter against a mock of its native API.

use std::sync::Arc;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zorix_core::{
    ExecutionId, FinishReason, GenerationParams, Message, ModelErrorKind, ModelGateway, ModelRequest, ToolCallId,
    ToolCallRequest, ToolDefinition,
};
use zorix_providers::{Catalog, HttpClient, HttpConfig, OllamaGateway, OllamaOptions, validate_base_url};

fn lines(values: &[Value]) -> String {
    values.iter().map(|v| format!("{v}\n")).collect()
}

fn gateway(server: &MockServer, options: OllamaOptions) -> OllamaGateway {
    let http = HttpClient::new("ollama", HttpConfig::default()).unwrap();
    OllamaGateway::new(
        "ollama",
        http,
        &validate_base_url(&server.uri()).unwrap(),
        Arc::new(Catalog::embedded()),
        options,
    )
}

fn tools() -> Vec<ToolDefinition> {
    ["math.evaluate"]
        .into_iter()
        .map(|id| ToolDefinition { id: id.into(), description: "calc".into(), input_schema: json!({"type": "object"}) })
        .collect()
}

fn request(messages: Vec<Message>) -> ModelRequest {
    ModelRequest {
        model: "qwen3:8b".into(),
        execution_id: ExecutionId::new(),
        messages,
        tools: tools(),
        ..ModelRequest::default()
    }
}

fn base() -> Vec<Message> {
    vec![Message::system("You are inside ZORIX."), Message::user("What is 17*23?")]
}

async fn call(g: &OllamaGateway, r: ModelRequest) -> Result<zorix_core::ModelResponse, zorix_core::ModelError> {
    g.generate(r, CancellationToken::new()).await
}

async fn mount_chat(server: &MockServer, body: String) {
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/x-ndjson"))
        .mount(server)
        .await;
}

fn done(extra: Value) -> Value {
    let mut base = json!({"model": "qwen3:8b", "message": {"role": "assistant", "content": ""}, "done": true, "done_reason": "stop", "prompt_eval_count": 42, "eval_count": 7});
    base.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    base
}

/// The shape of `/api/show` for a chat model with tools and thinking.
fn show_chat() -> Value {
    json!({
        "details": {"format": "gguf", "family": "qwen3", "parameter_size": "8.2B", "quantization_level": "Q4_K_M"},
        "model_info": {"general.architecture": "qwen3", "general.parameter_count": 8190735360u64, "qwen3.context_length": 40960, "qwen3.embedding_length": 4096},
        "capabilities": ["completion", "tools", "thinking"]
    })
}

#[tokio::test]
async fn a_text_answer_is_assembled_from_json_lines_with_usage() {
    let server = MockServer::start().await;
    mount_chat(&server, lines(&[
        json!({"model": "qwen3:8b", "message": {"role": "assistant", "content": "<think>hm</think>It is "}, "done": false}),
        json!({"model": "qwen3:8b", "message": {"role": "assistant", "content": "391."}, "done": false}),
        done(json!({})),
    ])).await;
    let response = call(&gateway(&server, OllamaOptions::default()), request(base())).await.unwrap();
    assert_eq!(response.text, "It is 391.");
    assert_eq!((response.finish, response.provider.as_deref()), (FinishReason::Stop, Some("ollama")));
    assert_eq!((response.usage.input_tokens, response.usage.output_tokens, response.usage.cost_micros), (42, 7, 0));
}

#[tokio::test]
async fn the_request_fixes_the_context_window_and_has_one_system_message() {
    let server = MockServer::start().await;
    mount_chat(&server, lines(&[done(json!({}))])).await;
    let g = gateway(&server, OllamaOptions::default());
    let mut r = request(base());
    r.params = GenerationParams { max_output_tokens: Some(300), effort: None };
    call(&g, r).await.unwrap();
    let body: Value = serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    assert_eq!(
        body["options"],
        json!({"num_ctx": 8192, "num_predict": 300}),
        "an unknown model gets the default context"
    );
    assert_eq!(body["stream"], true);
    assert_eq!(body["messages"][0], json!({"role": "system", "content": "You are inside ZORIX."}));
    assert_eq!(body["tools"][0]["function"]["name"], "math_evaluate");
}

#[tokio::test]
async fn tool_calls_arrive_with_object_arguments_and_get_generated_ids() {
    let server = MockServer::start().await;
    mount_chat(&server, lines(&[
        json!({"model": "qwen3:8b", "message": {"role": "assistant", "content": "", "tool_calls": [{"function": {"name": "math_evaluate", "arguments": {"expression": "17*23"}}}]}, "done": false}),
        done(json!({})),
    ])).await;
    let response = call(&gateway(&server, OllamaOptions::default()), request(base())).await.unwrap();
    assert_eq!(response.finish, FinishReason::ToolCalls);
    assert_eq!(
        (response.tool_calls[0].tool.as_str(), response.tool_calls[0].arguments.clone()),
        ("math.evaluate", json!({"expression": "17*23"}))
    );
    assert_eq!(response.tool_calls[0].provider_id.as_deref(), Some("ollama_0"));

    server.reset().await;
    mount_chat(&server, lines(&[done(json!({}))])).await;
    let calls = vec![ToolCallRequest {
        id: ToolCallId("call_1_0".into()),
        provider_id: Some("ollama_0".into()),
        tool: "math.evaluate".into(),
        arguments: json!({"expression": "17*23"}),
    }];
    let mut messages = base();
    messages.push(Message::assistant("", calls));
    messages.push(Message::tool_result(ToolCallId("call_1_0".into()), "{\"result\":391}", false));
    call(&gateway(&server, OllamaOptions::default()), request(messages)).await.unwrap();
    let body: Value = serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    assert_eq!(
        body["messages"][2]["tool_calls"][0]["function"],
        json!({"name": "math_evaluate", "arguments": {"expression": "17*23"}})
    );
    assert_eq!(
        body["messages"][3],
        json!({"role": "tool", "tool_name": "math_evaluate", "content": "{\"result\":391}"})
    );
}

#[tokio::test]
async fn discovery_learns_the_context_and_capabilities_and_the_runtime_is_told_the_real_window() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"models": [{"name": "qwen3:8b"}, {"name": "nomic-embed-text"}, {"name": "broken"}]}),
            ),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/show"))
        .and(wiremock::matchers::body_string_contains("qwen3:8b"))
        .respond_with(ResponseTemplate::new(200).set_body_json(show_chat()))
        .mount(&server)
        .await;
    Mock::given(method("POST")).and(path("/api/show")).and(wiremock::matchers::body_string_contains("nomic")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"details": {"parameter_size": "137M"}, "model_info": {"nomic-bert.context_length": 2048}, "capabilities": ["embedding"]}))).mount(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/show"))
        .and(wiremock::matchers::body_string_contains("broken"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let g = gateway(&server, OllamaOptions::default());
    assert!(g.describe("qwen3:8b").is_none(), "nothing is known before discovery");
    let found = g.refresh(&CancellationToken::new()).await.unwrap();
    assert_eq!(found.len(), 1, "the embedding model and the broken one are left out");
    let model = &found[0];
    assert_eq!(
        (model.id.as_str(), model.context_window, model.tools, model.thinking, model.vision, model.assumed),
        ("qwen3:8b", 32_768, true, true, false, false) // 40 960 on disk, capped by max_context
    );
    assert_eq!(model.tier, 2);
    let profile = g.describe("qwen3:8b").unwrap();
    assert_eq!(profile.context_window, 32_768, "capped by max_context, because that is what will be loaded");
    assert!(profile.tools);

    // The window asked of Ollama is the same one the runtime plans with.
    mount_chat(&server, lines(&[done(json!({}))])).await;
    call(&g, request(base())).await.unwrap();
    let chat = server.received_requests().await.unwrap().into_iter().find(|r| r.url.path() == "/api/chat").unwrap();
    let body: Value = serde_json::from_slice(&chat.body).unwrap();
    assert_eq!(body["options"]["num_ctx"], 32_768);
}

#[tokio::test]
async fn errors_are_typed_and_a_server_that_is_off_says_how_to_start_it() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"error": "model 'qwen3:8b' not found"})))
        .mount(&server)
        .await;
    let error = call(&gateway(&server, OllamaOptions::default()), request(base())).await.unwrap_err();
    assert_eq!(error.kind(), ModelErrorKind::NotFound);
    assert!(error.to_string().contains("not found"), "{error}");

    server.reset().await;
    mount_chat(&server, lines(&[json!({"error": "model requires more system memory"})])).await;
    assert_eq!(
        call(&gateway(&server, OllamaOptions::default()), request(base())).await.unwrap_err().kind(),
        ModelErrorKind::Unavailable
    );

    server.reset().await;
    mount_chat(
        &server,
        lines(&[json!({"model": "m", "message": {"role": "assistant", "content": "half"}, "done": false})]),
    )
    .await;
    let error = call(&gateway(&server, OllamaOptions::default()), request(base())).await.unwrap_err();
    assert!(error.to_string().contains("ended before it was complete"));

    let dead = OllamaGateway::new(
        "ollama",
        HttpClient::new("ollama", HttpConfig::default()).unwrap(),
        &validate_base_url("http://127.0.0.1:9").unwrap(),
        Arc::new(Catalog::embedded()),
        OllamaOptions::default(),
    );
    let error = call(&dead, request(base())).await.unwrap_err();
    assert_eq!(error.kind(), ModelErrorKind::Unavailable);
    assert!(error.to_string().contains("ollama serve"), "{error}");
}

#[tokio::test]
async fn a_length_stop_is_reported_and_the_last_line_without_a_newline_is_still_read() {
    let server = MockServer::start().await;
    let body = format!(
        "{}\n{}",
        json!({"model": "m", "message": {"role": "assistant", "content": "cut"}, "done": false}),
        done(json!({"done_reason": "length"}))
    );
    mount_chat(&server, body).await;
    let response = call(&gateway(&server, OllamaOptions::default()), request(base())).await.unwrap();
    assert_eq!((response.finish, response.text.as_str()), (FinishReason::Length, "cut"));
}
