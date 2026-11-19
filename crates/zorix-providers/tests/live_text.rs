//! Live text: what each adapter hands to a sink while the answer is still arriving, and that the finished response is
//! unchanged by it.

use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zorix_core::{ExecutionId, Message, ModelGateway, ModelRequest, TextSink};
use zorix_providers::adapter::ToolNameMap;
use zorix_providers::anthropic::Aggregator;
use zorix_providers::gemini::GeminiAggregator;
use zorix_providers::openai_chat::ChatAggregator;
use zorix_providers::openai_responses::ResponsesAggregator;
use zorix_providers::sse::SseEvent;
use zorix_providers::{Catalog, HttpClient, HttpConfig, OllamaGateway, OllamaOptions, validate_base_url};

#[derive(Default)]
struct Pieces(Mutex<Vec<String>>);

impl Pieces {
    fn all(&self) -> Vec<String> {
        self.0.lock().clone()
    }
}

impl TextSink for Pieces {
    fn text(&self, delta: &str) {
        self.0.lock().push(delta.to_string());
    }
}

fn event(name: Option<&str>, data: Value) -> SseEvent {
    SseEvent { event: name.map(str::to_string), data: data.to_string() }
}

fn plain(data: &str) -> SseEvent {
    SseEvent { event: None, data: data.to_string() }
}

#[test]
fn anthropic_text_goes_out_as_it_arrives_and_thinking_does_not() {
    let pieces = Arc::new(Pieces::default());
    let mut a = Aggregator::new().with_sink(pieces.clone());
    for e in [
        event(
            Some("message_start"),
            json!({"type": "message_start", "message": {"model": "m", "usage": {"input_tokens": 1}}}),
        ),
        event(
            None,
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "thinking", "thinking": ""}}),
        ),
        event(
            None,
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": "hmm"}}),
        ),
        event(None, json!({"type": "content_block_start", "index": 1, "content_block": {"type": "text", "text": ""}})),
        event(None, json!({"type": "content_block_delta", "index": 1, "delta": {"type": "text_delta", "text": "Hel"}})),
        event(None, json!({"type": "content_block_delta", "index": 1, "delta": {"type": "text_delta", "text": ""}})),
        event(None, json!({"type": "content_block_delta", "index": 1, "delta": {"type": "text_delta", "text": "lo"}})),
        event(
            None,
            json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 2}}),
        ),
        event(None, json!({"type": "message_stop"})),
    ] {
        a.on_event(&e).unwrap();
    }
    assert_eq!(pieces.all(), ["Hel", "lo"]);
    assert_eq!(a.finish("m", &ToolNameMap::default()).unwrap().text, "Hello");
}

#[test]
fn chat_text_hides_think_tags_even_when_a_tag_is_split() {
    let pieces = Arc::new(Pieces::default());
    let mut a = ChatAggregator::new().with_sink(pieces.clone());
    for text in ["<thi", "nk>secret</th", "ink>The ", "answer <", "b>."] {
        a.on_event(&plain(&json!({"choices": [{"delta": {"content": text}}]}).to_string())).unwrap();
    }
    a.on_event(&plain(&json!({"choices": [{"delta": {}, "finish_reason": "stop"}]}).to_string())).unwrap();
    a.on_event(&plain("[DONE]")).unwrap();
    let live: String = pieces.all().concat();
    assert_eq!(live, "The answer <b>.");
    assert_eq!(a.finish("m", &ToolNameMap::default()).unwrap().text, live);
}

#[test]
fn responses_text_streams_from_output_text_deltas() {
    let pieces = Arc::new(Pieces::default());
    let mut a = ResponsesAggregator::new().with_sink(pieces.clone());
    for e in [
        json!({"type": "response.output_text.delta", "delta": "Hi "}),
        json!({"type": "response.output_text.delta", "delta": "there"}),
        json!({"type": "response.completed", "response": {"model": "m", "status": "completed", "output": [{"type": "message", "content": [{"type": "output_text", "text": "Hi there"}]}], "usage": {"input_tokens": 1, "output_tokens": 2}}}),
    ] {
        a.on_event(&plain(&e.to_string())).unwrap();
    }
    assert_eq!(pieces.all(), ["Hi ", "there"]);
    assert_eq!(a.finish("m", &ToolNameMap::default()).unwrap().text, "Hi there");
}

#[test]
fn gemini_thoughts_stay_out_of_the_live_text() {
    let pieces = Arc::new(Pieces::default());
    let mut a = GeminiAggregator::new().with_sink(pieces.clone());
    for parts in [json!([{"text": "pondering", "thought": true}]), json!([{"text": "Four"}]), json!([{"text": "."}])] {
        a.on_event(&plain(&json!({"candidates": [{"content": {"parts": parts}}]}).to_string())).unwrap();
    }
    a.on_event(&plain(&json!({"candidates": [{"content": {"parts": []}, "finishReason": "STOP"}], "usageMetadata": {"promptTokenCount": 1, "candidatesTokenCount": 2}}).to_string())).unwrap();
    assert_eq!(pieces.all(), ["Four", "."]);
    assert_eq!(a.finish("m", &ToolNameMap::default()).unwrap().text, "Four.");
}

#[tokio::test]
async fn ollama_streams_lines_and_keeps_thinking_out() {
    let server = MockServer::start().await;
    let lines = [
        json!({"model": "qwen3:8b", "message": {"role": "assistant", "content": "<think>plan"}, "done": false}),
        json!({"model": "qwen3:8b", "message": {"role": "assistant", "content": "</think>Seven"}, "done": false}),
        json!({"model": "qwen3:8b", "message": {"role": "assistant", "content": "teen."}, "done": false}),
        json!({"model": "qwen3:8b", "message": {"role": "assistant", "content": ""}, "done": true, "done_reason": "stop", "prompt_eval_count": 4, "eval_count": 3}),
    ];
    let body: String = lines.iter().map(|l| format!("{l}\n")).collect();
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/x-ndjson"))
        .mount(&server)
        .await;
    let gateway = OllamaGateway::new(
        "ollama",
        HttpClient::new("ollama", HttpConfig::default()).unwrap(),
        &validate_base_url(&server.uri()).unwrap(),
        Arc::new(Catalog::embedded()),
        OllamaOptions::default(),
    );
    let request = ModelRequest {
        model: "qwen3:8b".into(),
        execution_id: ExecutionId::new(),
        messages: vec![Message::user("hi")],
        ..ModelRequest::default()
    };
    let pieces = Arc::new(Pieces::default());
    let response = gateway.generate_live(request, CancellationToken::new(), pieces.clone()).await.unwrap();
    assert_eq!(response.text, "Seventeen.");
    assert_eq!(pieces.all().concat(), "Seventeen.");
    assert!(pieces.all().len() >= 2, "it arrived in pieces: {:?}", pieces.all());
}

#[tokio::test]
async fn a_gateway_that_cannot_stream_sends_its_whole_answer_once() {
    struct Plain;
    #[async_trait::async_trait]
    impl ModelGateway for Plain {
        async fn generate(
            &self,
            request: ModelRequest,
            _cancel: CancellationToken,
        ) -> Result<zorix_core::ModelResponse, zorix_core::ModelError> {
            Ok(zorix_core::ModelResponse::new(
                request.model,
                "all at once".to_string(),
                vec![],
                zorix_core::Usage::default(),
                zorix_core::FinishReason::Stop,
            ))
        }
    }
    let pieces = Arc::new(Pieces::default());
    let response = Plain
        .generate_live(
            ModelRequest { model: "m".into(), ..ModelRequest::default() },
            CancellationToken::new(),
            pieces.clone(),
        )
        .await
        .unwrap();
    assert_eq!((pieces.all(), response.text.as_str()), (vec!["all at once".to_string()], "all at once"));
}
