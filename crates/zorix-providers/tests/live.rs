//! Tests against real providers. Every one is `#[ignore]`d: run them with `tools/live-tests.sh`.
//!
//! * Ollama needs a running server and a tool-capable model (`ZORIX_LIVE_OLLAMA_MODEL`, default `qwen3:1.7b`).
//! * Cloud providers run only when a key exists (`zorix provider set-key <name>` or the usual environment variable);
//!   without one the test says so and passes. Models come from `ZORIX_LIVE_<NAME>_MODEL`. Cloud tests cost a few cents.
//!
//! Nothing here prints a key or a whole answer; only lengths, tool names and token counts.

use std::sync::Arc;

use futures_util::StreamExt;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use zorix_core::{
    ExecutionEngine, ExecutionId, ExecutionState, Grants, Message, ModelGateway, ModelRequest, NewExecution,
    Permission, TaskRequest, ToolDefinition,
};
use zorix_providers::{ProvidersConfig, Router, build_router, discover_all, standard_secrets};
use zorix_runtime::{Engine, EngineConfig, InMemoryToolRegistry, MemoryRepository};
use zorix_tools::{BuiltinConfig, register_builtin};

fn router() -> Arc<Router> {
    let config = ProvidersConfig::default();
    let secrets = standard_secrets(&config);
    Arc::new(build_router(&config, secrets, None, false).expect("router"))
}

fn model_env(name: &str, default: &str) -> String {
    std::env::var(format!("ZORIX_LIVE_{name}_MODEL")).unwrap_or_else(|_| default.to_string())
}

fn add_tool() -> ToolDefinition {
    ToolDefinition {
        id: "math.add".into(),
        description: "Add two numbers and return the sum.".into(),
        input_schema: json!({"type": "object", "properties": {"a": {"type": "number"}, "b": {"type": "number"}}, "required": ["a", "b"]}),
    }
}

fn request(model: &str, text: &str, tools: Vec<ToolDefinition>) -> ModelRequest {
    ModelRequest {
        model: model.into(),
        execution_id: ExecutionId::new(),
        messages: vec![Message::user(text)],
        tools,
        ..ModelRequest::default()
    }
}

/// A plain answer, then a tool call, against `provider/model`.
async fn exercise(router: &Router, reference: &str) {
    let cancel = CancellationToken::new();
    let plain = router
        .generate(request(reference, "Reply with one short sentence about the sea.", vec![]), cancel.clone())
        .await
        .unwrap_or_else(|e| panic!("{reference}: {e}"));
    assert!(!plain.text.trim().is_empty(), "{reference}: an empty answer");
    assert!(plain.usage.output_tokens > 0, "{reference}: no usage was reported");
    eprintln!(
        "{reference}: answered with {} characters, {} output tokens",
        plain.text.len(),
        plain.usage.output_tokens
    );

    let asked = router
        .generate(
            request(
                reference,
                "Use the math.add tool to add 17 and 25. Do not answer without calling it.",
                vec![add_tool()],
            ),
            cancel,
        )
        .await
        .unwrap_or_else(|e| panic!("{reference}: {e}"));
    let call = asked.tool_calls.first().unwrap_or_else(|| panic!("{reference}: no tool call was made"));
    assert_eq!(call.tool, "math.add", "the tool name is mapped back to the id");
    assert_eq!(call.arguments["a"].as_f64().zip(call.arguments["b"].as_f64()), Some((17.0, 25.0)));
    eprintln!("{reference}: asked for {} with valid arguments", call.tool);
}

#[tokio::test]
#[ignore = "needs a running Ollama and a model"]
async fn ollama_discovery_plain_answer_and_tool_call() {
    let model = model_env("OLLAMA", "qwen3:1.7b");
    let router = router();
    let found = discover_all(&router, &CancellationToken::new()).await;
    let ollama = found.iter().find(|d| d.provider == "ollama").expect("ollama is built in");
    let models = ollama.models.as_ref().unwrap_or_else(|e| panic!("Ollama: {e}"));
    let info = models.iter().find(|m| m.id == model).unwrap_or_else(|| panic!("`ollama pull {model}` first"));
    assert!(info.tools, "{model} must support tools");
    assert!(info.context_window >= 2_048);
    exercise(&router, &format!("ollama/{model}")).await;
}

#[tokio::test]
#[ignore = "needs a running Ollama and a model"]
async fn ollama_runs_a_whole_execution_with_a_real_tool_loop() {
    let model = model_env("OLLAMA", "qwen3:1.7b");
    let dir = tempfile::tempdir().unwrap();
    let registry = Arc::new(InMemoryToolRegistry::new());
    register_builtin(&*registry, &BuiltinConfig { notes_file: dir.path().join("notes.json") }).unwrap();
    let engine = Engine::builder(Arc::new(MemoryRepository::new()), router())
        .tools(registry)
        .config(EngineConfig::default())
        .build();
    let created = engine
        .create(NewExecution {
            request: TaskRequest {
                prompt: "What is 17*23? Use the math.evaluate tool, then tell me the number.".into(),
                model: format!("ollama/{model}"),
                ..TaskRequest::default()
            },
            grants: Grants::none().with(Permission::use_tool("math.evaluate")),
            ..NewExecution::default()
        })
        .await
        .unwrap();
    let tenant = created.tenant.clone();
    let mut stream = engine.subscribe(&tenant, created.id, 0).await.unwrap();
    engine.start(&tenant, created.id).await.unwrap();
    let finished = tokio::time::timeout(std::time::Duration::from_secs(300), async {
        while let Some(event) = stream.next().await {
            if zorix_runtime::is_final(event.kind) {
                return;
            }
        }
    })
    .await;
    assert!(finished.is_ok(), "the execution did not finish in five minutes");
    let detail = engine.detail(&tenant, created.id).await.unwrap();
    assert_eq!(detail.execution.state, ExecutionState::Completed, "{:?}", detail.execution.failure);
    assert!(detail.tool_calls.iter().any(|c| c.tool == "math.evaluate"), "the model never used the tool");
    let answer = detail.execution.result.expect("a result").text;
    assert!(answer.contains("391"), "the answer should hold 391, got {} characters", answer.len());
    engine.shutdown().await;
}

async fn cloud(name: &str, default_model: &str) {
    let router = router();
    let status = zorix_providers::provider_statuses(router.config(), &standard_secrets(router.config()));
    let has_key = status.iter().any(|s| s.name == name && s.key == zorix_providers::KeyStatus::Present);
    if !has_key {
        eprintln!("{name}: no key stored (zorix provider set-key {name}), skipped");
        return;
    }
    let model = model_env(&name.to_uppercase(), default_model);
    exercise(&router, &format!("{name}/{model}")).await;
}

#[tokio::test]
#[ignore = "needs an Anthropic key and spends a little"]
async fn anthropic_plain_answer_and_tool_call() {
    cloud("anthropic", "claude-haiku-5-5").await;
}

#[tokio::test]
#[ignore = "needs an OpenAI key and spends a little"]
async fn openai_plain_answer_and_tool_call() {
    cloud("openai", "gpt-5-mini").await;
}

#[tokio::test]
#[ignore = "needs a Gemini key and spends a little"]
async fn gemini_plain_answer_and_tool_call() {
    cloud("gemini", "gemini-3-flash").await;
}

/// Embed two texts with a cloud model and check the vectors are usable.
async fn cloud_embeddings(name: &str, default_model: &str) {
    let router = router();
    let status = zorix_providers::provider_statuses(router.config(), &standard_secrets(router.config()));
    if !status.iter().any(|s| s.name == name && s.key == zorix_providers::KeyStatus::Present) {
        eprintln!("{name}: no key stored (zorix provider set-key {name}), skipped");
        return;
    }
    let model = std::env::var(format!("ZORIX_LIVE_{}_EMBED_MODEL", name.to_uppercase()))
        .unwrap_or_else(|_| default_model.to_string());
    let request = zorix_core::EmbeddingRequest {
        model: format!("{name}/{model}"),
        inputs: vec!["Deploys reach staging first.".into(), "The cafeteria serves lunch at noon.".into()],
        purpose: zorix_core::EmbedPurpose::Document,
        dimensions: None,
    };
    let response = router.embed(request, CancellationToken::new()).await.unwrap_or_else(|e| panic!("{name}: {e}"));
    assert_eq!(response.vectors.len(), 2);
    assert!(response.vectors[0].len() >= 256 && response.vectors[0].len() == response.vectors[1].len());
    eprintln!("{name}/{model}: {} dimensions", response.vectors[0].len());
}

#[tokio::test]
#[ignore = "needs an OpenAI key and spends a very little"]
async fn openai_embeddings_work() {
    cloud_embeddings("openai", "text-embedding-3-small").await;
}

#[tokio::test]
#[ignore = "needs a Gemini key and spends a very little"]
async fn gemini_embeddings_work() {
    cloud_embeddings("gemini", "gemini-embedding-001").await;
}
