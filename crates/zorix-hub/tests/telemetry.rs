//! What leaves the computer when telemetry is on, seen from a collector that is a plain listener on loopback: spans of the runtime with their numbers,
//! and nothing a person or a model said; a header only when one was given; a dead collector costs a run nothing; and turning it off stops it
//! (ADR-0046). One test, because the subscriber is global to the process.

use std::sync::Arc;
use std::time::{Duration, Instant};

use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::any_value::Value;
use parking_lot::Mutex;
use prost::Message as _;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing_subscriber::layer::SubscriberExt;
use zorix_core::{
    ExecutionEngine, ExecutionState as S, Grants, NewExecution, Permission, TaskRequest, TenantId, ToolSpec,
};
use zorix_hub::telemetry;
use zorix_runtime::{Engine, EngineConfig, InMemoryToolRegistry, MemoryRepository};
use zorix_sim::{ScriptedModel, Turn};

/// What the collector received.
#[derive(Clone, Debug)]
struct Received {
    path: String,
    authorization: Option<String>,
    body: Vec<u8>,
}

/// A collector: it reads each request, keeps it and answers 200 with an empty protobuf.
async fn collector() -> (String, Arc<Mutex<Vec<Received>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let keep = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else { return };
            let keep = keep.clone();
            tokio::spawn(async move {
                let mut data = Vec::new();
                let mut buffer = [0u8; 8192];
                let header_end = loop {
                    let n = stream.read(&mut buffer).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    data.extend_from_slice(&buffer[..n]);
                    if let Some(at) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        break at + 4;
                    }
                };
                let head = String::from_utf8_lossy(&data[..header_end]).to_string();
                let length = head
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                    })
                    .unwrap_or(0);
                while data.len() < header_end + length {
                    let n = stream.read(&mut buffer).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    data.extend_from_slice(&buffer[..n]);
                }
                let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                let authorization = head
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("authorization:").map(|v| v.trim().to_string()));
                keep.lock().push(Received { path, authorization, body: data[header_end..].to_vec() });
                let _ = stream
                    .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: application/x-protobuf\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                    .await;
            });
        }
    });
    (format!("http://{address}"), seen)
}

async fn run_once(prompt: &str, tool_argument: &str, answer: &str) -> zorix_core::Execution {
    let registry = Arc::new(InMemoryToolRegistry::new());
    {
        use zorix_core::ToolRegistry;
        let mut spec = ToolSpec::new(
            "echo",
            "Echo",
            "echoes",
            json!({"type": "object", "properties": {"text": {"type": "string"}}, "additionalProperties": false}),
        );
        spec.annotations = zorix_core::ToolAnnotations::safe_read();
        registry.register(Arc::new(EchoTool(spec))).unwrap();
    }
    let model =
        Arc::new(ScriptedModel::new([Turn::call("echo", json!({"text": tool_argument})), Turn::Say(answer.into())]));
    let engine = Engine::builder(Arc::new(MemoryRepository::new()), model)
        .tools(registry)
        .config(EngineConfig { workers: 1, ..EngineConfig::default() })
        .build();
    let new = NewExecution {
        request: TaskRequest { prompt: prompt.into(), model: "sim".into(), ..TaskRequest::default() },
        grants: Grants::none().with(Permission::use_tool("echo")),
        ..NewExecution::default()
    };
    let started = engine.submit(new).await.unwrap();
    let done = tokio::time::timeout(Duration::from_secs(10), engine.settle(&TenantId::local(), started.id))
        .await
        .unwrap()
        .unwrap();
    engine.shutdown().await;
    done
}

struct EchoTool(ToolSpec);

#[async_trait::async_trait]
impl zorix_core::Tool for EchoTool {
    fn spec(&self) -> &ToolSpec {
        &self.0
    }

    async fn call(
        &self,
        _context: &zorix_core::ToolContext,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, zorix_core::ToolError> {
        Ok(arguments)
    }
}

fn spans_of(received: &[Received]) -> Vec<opentelemetry_proto::tonic::trace::v1::Span> {
    received
        .iter()
        .flat_map(|r| ExportTraceServiceRequest::decode(r.body.as_slice()).unwrap().resource_spans)
        .flat_map(|r| r.scope_spans)
        .flat_map(|s| s.spans)
        .collect()
}

fn text_attribute(span: &opentelemetry_proto::tonic::trace::v1::Span, key: &str) -> Option<String> {
    span.attributes.iter().find(|a| a.key == key).and_then(|a| match a.value.as_ref()?.value.as_ref()? {
        Value::StringValue(s) => Some(s.clone()),
        Value::IntValue(i) => Some(i.to_string()),
        _ => None,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn spans_of_a_run_reach_a_collector_with_numbers_and_never_with_what_was_said() {
    let (slot, switch) = telemetry::channel();
    tracing::subscriber::set_global_default(tracing_subscriber::registry().with(slot)).unwrap();
    let (url, seen) = collector().await;
    assert!(!switch.is_on());

    // Off: nothing is sent.
    run_once("first", "a", "b").await;
    switch.stop().await;
    assert!(seen.lock().is_empty(), "something was sent while telemetry was off");

    // On, with a header. The run's prompt, tool argument and answer are things that must never leave.
    switch.apply(Some(&url), Some(("authorization".into(), "Bearer test-token".into()))).await.unwrap();
    assert!(switch.is_on());
    let done = run_once("PROMPT-SECRET-31337", "ARGUMENT-SECRET-31337", "ANSWER-SECRET-31337").await;
    assert_eq!(done.state, S::Completed, "{:?}", done.failure);
    // Another part of the program opens a span of its own and writes an event inside the run's span: neither may be exported.
    {
        let other = tracing::info_span!(target: "elsewhere", "elsewhere", detail = "OTHER-SPAN-SECRET-31337");
        let _guard = other.enter();
        tracing::info!(target: "zorix_runtime::run", "EVENT-SECRET-31337");
    }
    switch.stop().await; // sends what is queued

    let received = seen.lock().clone();
    assert!(!received.is_empty(), "nothing reached the collector");
    assert!(
        received.iter().all(|r| r.path == "/v1/traces"),
        "{:?}",
        received.iter().map(|r| &r.path).collect::<Vec<_>>()
    );
    assert!(
        received.iter().all(|r| r.authorization.as_deref() == Some("bearer test-token")),
        "the header goes with every request"
    );
    for r in &received {
        let text = String::from_utf8_lossy(&r.body);
        for secret in
            ["PROMPT-SECRET", "ARGUMENT-SECRET", "ANSWER-SECRET", "OTHER-SPAN-SECRET", "EVENT-SECRET", "test-token"]
        {
            assert!(!text.contains(secret), "{secret} left the computer");
        }
    }
    let spans = spans_of(&received);
    let names: Vec<&str> = spans.iter().map(|s| s.name.as_str()).collect();
    for wanted in ["invoke_agent", "chat", "execute_tool"] {
        assert!(names.contains(&wanted), "{wanted} missing in {names:?}");
    }
    assert!(!names.contains(&"elsewhere"));
    let agent = spans.iter().find(|s| s.name == "invoke_agent").unwrap();
    assert_eq!(text_attribute(agent, "zorix.execution.state").as_deref(), Some("completed"));
    assert!(
        text_attribute(agent, "zorix.usage.model_calls").is_some()
            && text_attribute(agent, "gen_ai.usage.input_tokens").is_some()
    );
    let tool = spans.iter().find(|s| s.name == "execute_tool").unwrap();
    assert_eq!(text_attribute(tool, "gen_ai.tool.name").as_deref(), Some("echo"));
    assert_eq!(text_attribute(tool, "zorix.tool.outcome").as_deref(), Some("ok"));
    assert!(
        spans.iter().any(|s| s.name == "chat" && text_attribute(s, "gen_ai.request.model").as_deref() == Some("sim"))
    );
    // Only what the runtime puts on a span: no attribute carries free text of a person.
    for span in &spans {
        for attribute in &span.attributes {
            assert!(
                attribute.key.starts_with("gen_ai.")
                    || attribute.key.starts_with("zorix.")
                    || attribute.key.starts_with("otel.")
                    || attribute.key.starts_with("busy_ns")
                    || attribute.key.starts_with("idle_ns")
                    || attribute.key.starts_with("code.")
                    || attribute.key.starts_with("thread.")
                    || attribute.key == "target",
                "unexpected attribute {}",
                attribute.key
            );
        }
    }

    // Turned off: nothing more is sent.
    let before = seen.lock().len();
    run_once("after", "c", "d").await;
    switch.stop().await;
    assert_eq!(seen.lock().len(), before, "something was sent after telemetry was turned off");

    // A collector that is not there costs a run nothing: it finishes at once and the program does not wait for the export.
    switch.apply(Some("http://127.0.0.1:1"), None).await.unwrap();
    let started = Instant::now();
    let done = run_once("dead collector", "e", "f").await;
    assert_eq!(done.state, S::Completed);
    assert!(started.elapsed() < Duration::from_secs(3), "the run waited for a dead collector: {:?}", started.elapsed());
    switch.stop().await;
}
