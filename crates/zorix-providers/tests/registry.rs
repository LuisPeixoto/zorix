//! The whole stack from `providers.toml`: router, adapters and mock servers.

use std::sync::Arc;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zorix_core::{ExecutionId, Message, ModelErrorKind, ModelGateway, ModelRequest, Secret, StepPhase};
use zorix_providers::secrets::{MemoryStore, SecretStore};
use zorix_providers::{KeyStatus, ProvidersConfig, build_router, discover_all, provider_statuses};
use zorix_sim::DemoModel;

fn config(anthropic: &MockServer, ollama: &MockServer, extra: &str) -> ProvidersConfig {
    ProvidersConfig::parse(&format!(
        r#"
[[provider]]
name = "anthropic"
kind = "anthropic"
base_url = "{}"

[[provider]]
name = "ollama"
kind = "ollama"
base_url = "{}"

[[provider]]
name = "openai"
kind = "openai"
base_url = "https://api.openai.com/v1"
{extra}
"#,
        anthropic.uri(),
        ollama.uri()
    ))
    .unwrap()
}

fn keys(names: &[&str]) -> Arc<dyn SecretStore> {
    let store = MemoryStore::new();
    for name in names {
        store.set(name, &Secret::new(format!("sk-{name}-0123456789abcdefghij"))).unwrap();
    }
    Arc::new(store)
}

fn request(model: &str) -> ModelRequest {
    ModelRequest {
        model: model.into(),
        execution_id: ExecutionId::new(),
        messages: vec![Message::user("hello")],
        ..ModelRequest::default()
    }
}

fn ollama_answer(text: &str) -> ResponseTemplate {
    let body = format!(
        "{}\n{}\n",
        json!({"model": "qwen3:8b", "message": {"role": "assistant", "content": text}, "done": false}),
        json!({"model": "qwen3:8b", "message": {"role": "assistant", "content": ""}, "done": true, "done_reason": "stop", "prompt_eval_count": 5, "eval_count": 3})
    );
    ResponseTemplate::new(200).set_body_raw(body, "application/x-ndjson")
}

const ROUTE: &str = r#"
[[route]]
name = "r"
candidates = ["anthropic/claude-opus-5-5", "ollama/qwen3:8b"]
"#;

#[tokio::test]
async fn a_route_from_the_file_falls_back_from_a_failing_cloud_provider_to_a_local_one() {
    let (cloud, local) = (MockServer::start().await, MockServer::start().await);
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(529)
                .set_body_json(json!({"error": {"type": "overloaded_error", "message": "Overloaded"}})),
        )
        .mount(&cloud)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ollama_answer("hello from home"))
        .mount(&local)
        .await;

    let router = build_router(&config(&cloud, &local, ROUTE), keys(&["anthropic"]), None, false).unwrap();
    let response = router.generate(request("route:r"), CancellationToken::new()).await.unwrap();
    assert_eq!((response.text.as_str(), response.provider.as_deref()), ("hello from home", Some("ollama")));
    let attempts: Vec<(String, String)> =
        response.route.unwrap().attempts.into_iter().map(|a| (a.provider, a.outcome)).collect();
    assert_eq!(
        attempts,
        vec![("anthropic".to_string(), "unavailable".to_string()), ("ollama".to_string(), "ok".to_string())]
    );
    assert_eq!(response.usage.input_tokens, 5);
}

#[tokio::test]
async fn a_missing_key_is_an_authentication_error_and_the_route_moves_on_without_calling_the_provider() {
    let (cloud, local) = (MockServer::start().await, MockServer::start().await);
    Mock::given(method("POST")).and(path("/api/chat")).respond_with(ollama_answer("local")).mount(&local).await;
    let router = build_router(&config(&cloud, &local, ROUTE), keys(&[]), None, false).unwrap();
    let response = router.generate(request("route:r"), CancellationToken::new()).await.unwrap();
    assert_eq!(response.provider.as_deref(), Some("ollama"));
    assert_eq!(response.route.unwrap().attempts[0].outcome, "auth");
    assert!(cloud.received_requests().await.unwrap().is_empty(), "no key, no request");
}

#[tokio::test]
async fn the_default_is_anthropics_model_when_there_is_a_key_and_otherwise_the_user_must_choose() {
    let (cloud, local) = (MockServer::start().await, MockServer::start().await);
    let with = build_router(&config(&cloud, &local, ""), keys(&["anthropic"]), None, false).unwrap();
    assert_eq!(with.resolve("default", StepPhase::Start).unwrap().candidates[0].model, "claude-opus-5-5");
    let without = build_router(&config(&cloud, &local, ""), keys(&[]), None, false).unwrap();
    assert!(without.resolve("default", StepPhase::Start).unwrap_err().contains("No default model"));
    let set =
        build_router(&config(&cloud, &local, "\n[alias]\ndefault = \"ollama/qwen3:8b\"\n"), keys(&[]), None, false)
            .unwrap();
    assert_eq!(set.resolve("default", StepPhase::Start).unwrap().candidates[0].provider, "ollama");
}

#[tokio::test]
async fn local_only_never_touches_the_cloud() {
    let local = MockServer::start().await;
    Mock::given(method("POST")).and(path("/api/chat")).respond_with(ollama_answer("private")).mount(&local).await;
    // Mock servers sit on loopback, so the cloud provider keeps its real (remote) address and is never reached.
    let cfg = ProvidersConfig::parse(&format!(
        "[[provider]]\nname = \"anthropic\"\nkind = \"anthropic\"\n[[provider]]\nname = \"ollama\"\nkind = \"ollama\"\nbase_url = \"{}\"\n{ROUTE}",
        local.uri()
    ))
    .unwrap();
    let router = build_router(&cfg, keys(&["anthropic"]), None, true).unwrap();
    assert_eq!(
        router.generate(request("route:r"), CancellationToken::new()).await.unwrap().provider.as_deref(),
        Some("ollama")
    );
    let error = router.generate(request("anthropic/claude-opus-5-5"), CancellationToken::new()).await.unwrap_err();
    assert!(error.to_string().contains("local only"), "{error}");
}

#[tokio::test]
async fn the_simulated_models_are_a_provider_and_demo_is_still_an_alias() {
    let (cloud, local) = (MockServer::start().await, MockServer::start().await);
    let router = build_router(&config(&cloud, &local, ""), keys(&[]), Some(Arc::new(DemoModel::new())), false).unwrap();
    let mut r = request("demo");
    r.messages = vec![Message::user("Tell me a joke")];
    let response = router.generate(r, CancellationToken::new()).await.unwrap();
    assert_eq!(response.provider.as_deref(), Some("sim"));
    assert!(response.text.contains("no tool"));
    assert!(router.generate(request("sim/demo"), CancellationToken::new()).await.is_ok());
}

#[tokio::test]
async fn a_disabled_provider_is_skipped_and_a_duplicate_name_is_refused() {
    let (cloud, local) = (MockServer::start().await, MockServer::start().await);
    Mock::given(method("POST")).and(path("/api/chat")).respond_with(ollama_answer("on")).mount(&local).await;
    let cfg = ProvidersConfig::parse(&format!(
        "[[provider]]\nname = \"anthropic\"\nkind = \"anthropic\"\nbase_url = \"{}\"\nenabled = false\n[[provider]]\nname = \"ollama\"\nkind = \"ollama\"\nbase_url = \"{}\"\n{ROUTE}",
        cloud.uri(),
        local.uri()
    ))
    .unwrap();
    let router = build_router(&cfg, keys(&["anthropic"]), None, false).unwrap();
    let response = router.generate(request("route:r"), CancellationToken::new()).await.unwrap();
    assert_eq!(response.provider.as_deref(), Some("ollama"));
    assert!(cloud.received_requests().await.unwrap().is_empty());
    let twice = ProvidersConfig::parse(
        "[[provider]]\nname = \"a\"\nkind = \"openai\"\n[[provider]]\nname = \"a\"\nkind = \"gemini\"\n",
    );
    assert!(twice.unwrap_err().contains("listed twice"));
}

#[tokio::test]
async fn statuses_say_which_providers_have_what_they_need_without_showing_keys() {
    let (cloud, local) = (MockServer::start().await, MockServer::start().await);
    let cfg = config(&cloud, &local, "");
    let store = keys(&["anthropic"]);
    let statuses = provider_statuses(&cfg, &store);
    let by_name = |n: &str| statuses.iter().find(|s| s.name == n).unwrap().clone();
    assert_eq!(by_name("anthropic").key, KeyStatus::Present);
    assert_eq!(by_name("openai").key, KeyStatus::Missing);
    assert_eq!(by_name("ollama").key, KeyStatus::NotNeeded);
    assert!(by_name("ollama").local && !by_name("openai").local);
    assert!(!format!("{statuses:?}").contains("sk-anthropic"));
}

#[tokio::test]
async fn discovery_teaches_the_router_what_each_provider_offers() {
    let (cloud, local) = (MockServer::start().await, MockServer::start().await);
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [
        {"id": "claude-opus-5-5", "display_name": "Claude Opus 5.5", "max_input_tokens": 1000000, "max_tokens": 128000},
        {"id": "claude-future-9", "display_name": "Claude Future 9", "max_input_tokens": 500000, "max_tokens": 64000}
    ], "has_more": false})))
        .mount(&cloud)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [{"name": "qwen3:8b"}]})))
        .mount(&local)
        .await;
    Mock::given(method("POST")).and(path("/api/show")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"details": {"parameter_size": "8B"}, "model_info": {"qwen3.context_length": 40960}, "capabilities": ["completion", "tools"]}))).mount(&local).await;

    let router = build_router(&config(&cloud, &local, ""), keys(&["anthropic"]), None, false).unwrap();
    assert!(router.describe("anthropic/claude-future-9").is_none());
    let found = discover_all(&router, &CancellationToken::new()).await;
    let counts: Vec<(String, usize)> =
        found.iter().map(|d| (d.provider.clone(), d.models.as_ref().map(Vec::len).unwrap_or(99))).collect();
    assert!(counts.contains(&("anthropic".into(), 2)) && counts.contains(&("ollama".into(), 1)), "{counts:?}");
    let openai = found.iter().find(|d| d.provider == "openai").unwrap();
    assert!(
        openai.models.as_ref().unwrap_err().contains("zorix provider set-key openai"),
        "a provider without a key reports why: {openai:?}"
    );
    assert_eq!(router.describe("anthropic/claude-future-9").unwrap().context_window, 500_000);
    assert_eq!(router.describe("ollama/qwen3:8b").unwrap().context_window, 32_768, "capped to what Ollama will load");
    assert!(
        router.model_info("anthropic", "claude-opus-5-5").unwrap().price.is_some(),
        "the catalogue's price survives discovery"
    );
}

#[tokio::test]
async fn an_invalid_configuration_is_refused_when_the_router_is_built() {
    let (cloud, local) = (MockServer::start().await, MockServer::start().await);
    let mut cfg = config(&cloud, &local, "");
    cfg.alias.insert("x".into(), "nobody/model".into());
    let error = build_router(&cfg, keys(&[]), None, false).err().unwrap();
    assert!(error.contains("no provider called"), "{error}");
    let _: Value = json!(null);
    let _ = ModelErrorKind::Auth;
}
