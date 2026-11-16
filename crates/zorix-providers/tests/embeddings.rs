//! Embeddings through every adapter against mock servers: addresses, headers, batching, prefixes, normalising, errors,
//! and the router's rules (one explicit model, no fallback, local only).

use std::sync::Arc;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zorix_core::{EmbedPurpose, EmbeddingRequest, ModelErrorKind, ModelGateway, Secret};
use zorix_providers::secrets::{MemoryStore, SecretStore};
use zorix_providers::{
    Catalog, GeminiGateway, GeminiOptions, HttpClient, HttpConfig, OllamaGateway, OllamaOptions, OpenAiChatGateway,
    OpenAiChatOptions, OpenAiOptions, OpenAiResponsesGateway, ProvidersConfig, build_router, validate_base_url,
};

const KEY: &str = "sk-embed-0123456789abcdefghij";

fn request(model: &str, inputs: &[&str], purpose: EmbedPurpose) -> EmbeddingRequest {
    EmbeddingRequest {
        model: model.into(),
        inputs: inputs.iter().map(|s| s.to_string()).collect(),
        purpose,
        dimensions: None,
    }
}

fn keys(names: &[&str]) -> Arc<dyn SecretStore> {
    let store = MemoryStore::new();
    for name in names {
        store.set(name, &Secret::new(KEY)).unwrap();
    }
    Arc::new(store)
}

fn norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

async fn bodies(server: &MockServer) -> Vec<Value> {
    server.received_requests().await.unwrap().iter().map(|r| serde_json::from_slice(&r.body).unwrap()).collect()
}

fn ollama(server: &MockServer) -> OllamaGateway {
    let http = HttpClient::new("ollama", HttpConfig::default()).unwrap();
    OllamaGateway::new(
        "ollama",
        http,
        &validate_base_url(&server.uri()).unwrap(),
        Arc::new(Catalog::embedded()),
        OllamaOptions::default(),
    )
}

fn openai(server: &MockServer, store: Arc<dyn SecretStore>) -> OpenAiResponsesGateway {
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

fn compatible(server: &MockServer, store: Arc<dyn SecretStore>) -> OpenAiChatGateway {
    let http = HttpClient::new("lab", HttpConfig::default()).unwrap();
    let base = validate_base_url(&format!("{}/api/v1", server.uri())).unwrap();
    OpenAiChatGateway::new(
        "lab",
        http,
        &base,
        store,
        "lab",
        Arc::new(Catalog::embedded()),
        OpenAiChatOptions::default(),
    )
    .unwrap()
}

fn gemini(server: &MockServer, store: Arc<dyn SecretStore>) -> GeminiGateway {
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

// ---------------------------------------------------------------------------------------------------------------------
// Ollama

#[tokio::test]
async fn ollama_sends_a_batch_with_the_models_prefix_and_returns_unit_vectors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/embed"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"model": "nomic-embed-text", "embeddings": [[3.0, 4.0], [0.0, 2.0]], "prompt_eval_count": 11}),
        ))
        .mount(&server)
        .await;
    let g = ollama(&server);
    let mut r = request("nomic-embed-text", &["first text", "second text"], EmbedPurpose::Query);
    r.dimensions = Some(256);
    let response = g.embed(r, CancellationToken::new()).await.unwrap();
    assert_eq!(response.vectors.len(), 2);
    assert!(response.vectors.iter().all(|v| (norm(v) - 1.0).abs() < 1e-6), "vectors are normalised");
    assert_eq!((response.usage.input_tokens, response.provider.as_deref()), (11, Some("ollama")));
    let body = &bodies(&server).await[0];
    assert_eq!(body["input"], json!(["search_query: first text", "search_query: second text"]));
    assert_eq!(
        (body["model"].as_str(), body["truncate"].as_bool(), body["dimensions"].as_u64()),
        (Some("nomic-embed-text"), Some(true), Some(256))
    );
    // Documents get the other prefix.
    g.embed(request("nomic-embed-text", &["x"], EmbedPurpose::Document), CancellationToken::new()).await.unwrap_err();
    assert_eq!(bodies(&server).await[1]["input"], json!(["search_document: x"]));
}

#[tokio::test]
async fn ollama_splits_big_requests_and_keeps_the_order() {
    let server = MockServer::start().await;
    // Answers with one vector per input, encoding the batch position in the first component.
    struct PerInput;
    impl wiremock::Respond for PerInput {
        fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let rows: Vec<Value> = body["input"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| json!([t.as_str().unwrap().len() as f64, 1.0]))
                .collect();
            ResponseTemplate::new(200).set_body_json(json!({"embeddings": rows, "prompt_eval_count": rows.len()}))
        }
    }
    Mock::given(method("POST")).and(path("/api/embed")).respond_with(PerInput).mount(&server).await;
    let inputs: Vec<String> = (0..70).map(|i| "x".repeat(i + 1)).collect();
    let refs: Vec<&str> = inputs.iter().map(String::as_str).collect();
    let response = ollama(&server)
        .embed(request("all-minilm", &refs, EmbedPurpose::Document), CancellationToken::new())
        .await
        .unwrap();
    let sent = bodies(&server).await;
    assert_eq!(
        sent.iter().map(|b| b["input"].as_array().unwrap().len()).collect::<Vec<_>>(),
        vec![32, 32, 6],
        "all-minilm batches of 32"
    );
    assert_eq!(response.vectors.len(), 70);
    // The first component grows with the text length, so order is kept across batches.
    let firsts: Vec<f32> = response.vectors.iter().map(|v| v[0]).collect();
    assert!(firsts.windows(2).all(|w| w[0] < w[1]), "order was kept");
    assert_eq!(response.usage.input_tokens, 70);
}

#[tokio::test]
async fn ollama_reports_a_missing_model_and_an_empty_text_before_calling() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/embed"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_json(json!({"error": "model \"nope\" not found, try pulling it first"})),
        )
        .mount(&server)
        .await;
    let g = ollama(&server);
    let error = g.embed(request("nope", &["x"], EmbedPurpose::Document), CancellationToken::new()).await.unwrap_err();
    assert_eq!(error.kind(), ModelErrorKind::NotFound);
    let before = server.received_requests().await.unwrap().len();
    let empty =
        g.embed(request("nope", &["ok", "  "], EmbedPurpose::Document), CancellationToken::new()).await.unwrap_err();
    assert_eq!(empty.kind(), ModelErrorKind::Invalid);
    assert_eq!(server.received_requests().await.unwrap().len(), before, "nothing was sent");
}

#[tokio::test]
async fn ollama_refuses_an_answer_with_the_wrong_number_of_vectors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/embed"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"embeddings": [[1.0, 0.0]]})))
        .mount(&server)
        .await;
    let error = ollama(&server)
        .embed(request("all-minilm", &["a", "b"], EmbedPurpose::Document), CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ModelErrorKind::Unavailable);
    assert!(error.to_string().contains("1 vector(s) for 2"), "{error}");
}

#[tokio::test]
async fn ollama_lists_only_the_embedding_models_with_their_sizes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"models": [{"name": "all-minilm:latest"}, {"name": "qwen3:8b"}, {"name": "mystery:1b"}]}),
        ))
        .mount(&server)
        .await;
    struct Show;
    impl wiremock::Respond for Show {
        fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            ResponseTemplate::new(200).set_body_json(match body["model"].as_str().unwrap() {
                "all-minilm:latest" => json!({"capabilities": ["embedding"], "model_info": {"bert.embedding_length": 384, "bert.context_length": 512}}),
                "qwen3:8b" => json!({"capabilities": ["completion", "tools"], "model_info": {"qwen3.context_length": 40960}}),
                _ => json!({"capabilities": ["completion"]}),
            })
        }
    }
    Mock::given(method("POST")).and(path("/api/show")).respond_with(Show).mount(&server).await;
    let found = ollama(&server).embedding_models(&CancellationToken::new()).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(
        (found[0].id.as_str(), found[0].dimensions, found[0].max_input_tokens),
        ("all-minilm:latest", Some(384), Some(512))
    );
}

// ---------------------------------------------------------------------------------------------------------------------
// OpenAI and compatible servers

#[tokio::test]
async fn openai_sends_the_key_and_puts_vectors_back_in_input_order() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .and(header("authorization", format!("Bearer {KEY}").as_str()))
        .and(body_partial_json(
            json!({"model": "text-embedding-3-small", "encoding_format": "float", "dimensions": 512}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "text-embedding-3-small-2026",
            "data": [{"index": 1, "embedding": [0.0, 5.0]}, {"index": 0, "embedding": [2.0, 0.0]}],
            "usage": {"prompt_tokens": 9, "total_tokens": 9}
        })))
        .mount(&server)
        .await;
    let mut r = request("text-embedding-3-small", &["zero", "one"], EmbedPurpose::Document);
    r.dimensions = Some(512);
    let response = openai(&server, keys(&["openai"])).embed(r, CancellationToken::new()).await.unwrap();
    assert_eq!(response.vectors, vec![vec![1.0, 0.0], vec![0.0, 1.0]], "index 0 first, normalised");
    assert_eq!((response.usage.input_tokens, response.model.as_str()), (9, "text-embedding-3-small-2026"));
    // The key travels only in the header.
    for body in bodies(&server).await {
        assert!(!body.to_string().contains(KEY));
    }
}

#[tokio::test]
async fn openai_without_a_key_or_with_a_wrong_one_is_an_authentication_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(401).set_body_json(
            json!({"error": {"message": "Incorrect API key provided", "type": "invalid_request_error"}}),
        ))
        .mount(&server)
        .await;
    let g = openai(&server, keys(&[]));
    let missing = g
        .embed(request("text-embedding-3-small", &["x"], EmbedPurpose::Query), CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(missing.kind(), ModelErrorKind::Auth);
    assert!(missing.to_string().contains("zorix provider set-key openai"), "{missing}");
    assert!(server.received_requests().await.unwrap().is_empty(), "no key, no request");
    let wrong = openai(&server, keys(&["openai"]))
        .embed(request("text-embedding-3-small", &["x"], EmbedPurpose::Query), CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(wrong.kind(), ModelErrorKind::Auth);
    assert!(!wrong.to_string().contains(KEY));
}

#[tokio::test]
async fn openai_splits_at_sixty_four_texts() {
    let server = MockServer::start().await;
    struct Rows;
    impl wiremock::Respond for Rows {
        fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let rows: Vec<Value> = (0..body["input"].as_array().unwrap().len())
                .map(|i| json!({"index": i, "embedding": [1.0, 1.0]}))
                .collect();
            ResponseTemplate::new(200).set_body_json(json!({"data": rows, "usage": {"prompt_tokens": 1}}))
        }
    }
    Mock::given(method("POST")).and(path("/v1/embeddings")).respond_with(Rows).mount(&server).await;
    let texts: Vec<String> = (0..130).map(|i| format!("t{i}")).collect();
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let response = openai(&server, keys(&["openai"]))
        .embed(request("text-embedding-3-small", &refs, EmbedPurpose::Document), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(response.vectors.len(), 130);
    assert_eq!(
        bodies(&server).await.iter().map(|b| b["input"].as_array().unwrap().len()).collect::<Vec<_>>(),
        vec![64, 64, 2]
    );
    assert_eq!(response.usage.input_tokens, 3);
}

#[tokio::test]
async fn a_compatible_server_is_asked_at_its_own_embeddings_address() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"embedding": [0.0, 3.0]}]})))
        .mount(&server)
        .await;
    // A loopback server needs no key.
    let response = compatible(&server, keys(&[]))
        .embed(request("any-model", &["x"], EmbedPurpose::Document), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(response.vectors, vec![vec![0.0, 1.0]]);
    assert_eq!(response.provider.as_deref(), Some("lab"));
}

// ---------------------------------------------------------------------------------------------------------------------
// Gemini

#[tokio::test]
async fn gemini_asks_for_the_right_task_and_normalises_reduced_vectors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/models/gemini-embedding-001:batchEmbedContents"))
        .and(header("x-goog-api-key", KEY))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"embeddings": [{"values": [3.0, 4.0]}, {"values": [0.0, 9.0]}]})),
        )
        .mount(&server)
        .await;
    let g = gemini(&server, keys(&["gemini"]));
    let mut r = request("gemini-embedding-001", &["a", "b"], EmbedPurpose::Query);
    r.dimensions = Some(768);
    let response = g.embed(r, CancellationToken::new()).await.unwrap();
    assert_eq!(response.vectors.len(), 2);
    assert!(
        response.vectors.iter().all(|v| (norm(v) - 1.0).abs() < 1e-6),
        "reduced sizes arrive unnormalised and are fixed here"
    );
    let body = &bodies(&server).await[0];
    let first = &body["requests"][0];
    assert_eq!(
        (first["taskType"].as_str(), first["outputDimensionality"].as_u64(), first["model"].as_str()),
        (Some("RETRIEVAL_QUERY"), Some(768), Some("models/gemini-embedding-001"))
    );
    assert_eq!(first["content"]["parts"][0]["text"], "a");
    g.embed(request("models/gemini-embedding-001", &["a"], EmbedPurpose::Document), CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(
        bodies(&server).await[1]["requests"][0]["taskType"],
        "RETRIEVAL_DOCUMENT",
        "a models/ prefix is accepted"
    );
}

#[tokio::test]
async fn gemini_maps_a_refused_key_to_an_authentication_error_and_splits_at_fifty() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": {"code": 400, "message": "API key not valid. Please pass a valid API key.", "status": "INVALID_ARGUMENT"}}))).mount(&server).await;
    let error = gemini(&server, keys(&["gemini"]))
        .embed(request("gemini-embedding-001", &["x"], EmbedPurpose::Document), CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ModelErrorKind::Auth);
    let server = MockServer::start().await;
    struct Rows;
    impl wiremock::Respond for Rows {
        fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let rows: Vec<Value> =
                body["requests"].as_array().unwrap().iter().map(|_| json!({"values": [1.0, 0.0]})).collect();
            ResponseTemplate::new(200).set_body_json(json!({"embeddings": rows}))
        }
    }
    Mock::given(method("POST")).respond_with(Rows).mount(&server).await;
    let texts: Vec<String> = (0..120).map(|i| format!("t{i}")).collect();
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let response = gemini(&server, keys(&["gemini"]))
        .embed(request("gemini-embedding-001", &refs, EmbedPurpose::Document), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(response.vectors.len(), 120);
    assert_eq!(
        bodies(&server).await.iter().map(|b| b["requests"].as_array().unwrap().len()).collect::<Vec<_>>(),
        vec![50, 50, 20]
    );
}

// ---------------------------------------------------------------------------------------------------------------------
// Profiles and the router

#[tokio::test]
async fn gateways_report_what_they_know_about_their_embedding_models() {
    let server = MockServer::start().await;
    let profile = openai(&server, keys(&[])).embedding_profile("text-embedding-3-large").unwrap();
    assert_eq!((profile.dimensions, profile.max_batch), (3072, 64));
    assert_eq!(gemini(&server, keys(&[])).embedding_profile("gemini-embedding-001").unwrap().max_batch, 50);
    assert_eq!(ollama(&server).embedding_profile("all-minilm:latest").unwrap().dimensions, 384);
    assert!(ollama(&server).embedding_profile("llama3").is_none());
    // Anthropic has no embeddings at all.
    let anthropic = zorix_providers::AnthropicGateway::new(
        "anthropic",
        HttpClient::new("anthropic", HttpConfig::default()).unwrap(),
        &validate_base_url(&server.uri()).unwrap(),
        keys(&["anthropic"]),
        "anthropic",
        Arc::new(Catalog::embedded()),
        zorix_providers::AnthropicOptions::default(),
    )
    .unwrap();
    let error = anthropic.embed(request("x", &["a"], EmbedPurpose::Query), CancellationToken::new()).await.unwrap_err();
    assert_eq!(error.kind(), ModelErrorKind::Unsupported);
    assert!(anthropic.embedding_profile("x").is_none());
}

fn router_config(local: &MockServer, extra: &str) -> ProvidersConfig {
    ProvidersConfig::parse(&format!(
        "[[provider]]\nname = \"ollama\"\nkind = \"ollama\"\nbase_url = \"{}\"\n\n[[provider]]\nname = \"openai\"\nkind = \"openai\"\n{extra}\n[alias]\nembed = \"ollama/all-minilm\"\n\n[[route]]\nname = \"r\"\ncandidates = [\"ollama/all-minilm\", \"openai/text-embedding-3-small\"]\n",
        local.uri()
    ))
    .unwrap()
}

#[tokio::test]
async fn the_router_embeds_with_exactly_one_model_and_never_falls_back() {
    let local = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/embed"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"embeddings": [[1.0, 0.0]], "prompt_eval_count": 2})),
        )
        .mount(&local)
        .await;
    let router = build_router(&router_config(&local, ""), keys(&["openai"]), None, false).unwrap();
    let response = router
        .embed(request("ollama/all-minilm", &["x"], EmbedPurpose::Document), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!((response.provider.as_deref(), response.model.as_str()), (Some("ollama"), "all-minilm"));
    // An alias that names one model works; a route does not, however it is ordered.
    router.embed(request("embed", &["x"], EmbedPurpose::Document), CancellationToken::new()).await.unwrap();
    let route =
        router.embed(request("route:r", &["x"], EmbedPurpose::Document), CancellationToken::new()).await.unwrap_err();
    assert_eq!(route.kind(), ModelErrorKind::Invalid);
    assert!(route.to_string().contains("cannot be compared"), "{route}");
    for bad in ["nobody/x", "default"] {
        assert_eq!(
            router
                .embed(request(bad, &["x"], EmbedPurpose::Document), CancellationToken::new())
                .await
                .unwrap_err()
                .kind(),
            ModelErrorKind::Invalid,
            "{bad}"
        );
    }
    assert_eq!(router.embedding_profile("ollama/all-minilm").unwrap().dimensions, 384);
    assert!(router.embedding_profile("route:r").is_none());

    // The chosen model failing is the answer: there is no other model to try.
    let broken = MockServer::start().await;
    Mock::given(method("POST")).and(path("/api/embed")).respond_with(ResponseTemplate::new(503)).mount(&broken).await;
    let working = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"embedding": [1.0, 0.0]}]})))
        .mount(&working)
        .await;
    let config = ProvidersConfig::parse(&format!(
        "[[provider]]\nname = \"ollama\"\nkind = \"ollama\"\nbase_url = \"{}\"\n\n[[provider]]\nname = \"openai\"\nkind = \"openai\"\nbase_url = \"{}/v1\"\n",
        broken.uri(),
        working.uri()
    ))
    .unwrap();
    let router = build_router(&config, keys(&["openai"]), None, false).unwrap();
    let error = router
        .embed(request("ollama/all-minilm", &["x"], EmbedPurpose::Document), CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ModelErrorKind::Unavailable);
    assert!(working.received_requests().await.unwrap().is_empty(), "no fallback to the other provider");
}

#[tokio::test]
async fn local_only_keeps_embeddings_off_the_cloud() {
    let local = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/embed"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"embeddings": [[1.0, 0.0]]})))
        .mount(&local)
        .await;
    // The cloud provider keeps its real (remote) address and is never reached.
    let router = build_router(&router_config(&local, ""), keys(&["openai"]), None, true).unwrap();
    router.embed(request("ollama/all-minilm", &["x"], EmbedPurpose::Document), CancellationToken::new()).await.unwrap();
    let cloud = router
        .embed(
            request("openai/text-embedding-3-small", &["secret document text"], EmbedPurpose::Document),
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(cloud.kind(), ModelErrorKind::Invalid);
    assert!(cloud.to_string().contains("local only"), "{cloud}");
}

#[tokio::test]
async fn a_turned_off_provider_is_not_used_for_embeddings() {
    let local = MockServer::start().await;
    let config = ProvidersConfig::parse(&format!(
        "[[provider]]\nname = \"ollama\"\nkind = \"ollama\"\nbase_url = \"{}\"\nenabled = false\n",
        local.uri()
    ))
    .unwrap();
    let router = build_router(&config, keys(&[]), None, false).unwrap();
    let error = router
        .embed(request("ollama/all-minilm", &["x"], EmbedPurpose::Document), CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ModelErrorKind::Invalid);
    assert!(local.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn the_simulated_embeddings_work_through_the_router() {
    let local = MockServer::start().await;
    let router =
        build_router(&router_config(&local, ""), keys(&[]), Some(Arc::new(zorix_sim::DemoModel::new())), false)
            .unwrap();
    let response = router
        .embed(
            request("sim/hash-embed", &["deploys reach staging", "lunch at noon"], EmbedPurpose::Document),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!((response.provider.as_deref(), response.vectors.len(), response.vectors[0].len()), (Some("sim"), 2, 64));
    assert_eq!(router.embedding_profile("sim/hash-embed").unwrap().dimensions, 64);
}

#[tokio::test]
async fn the_choices_are_the_installed_local_models_and_the_cloud_ones_with_a_key() {
    let local = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"models": [{"name": "all-minilm:latest"}, {"name": "qwen3:8b"}]})),
        )
        .mount(&local)
        .await;
    struct Show;
    impl wiremock::Respond for Show {
        fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            ResponseTemplate::new(200).set_body_json(if body["model"] == "all-minilm:latest" {
                json!({"capabilities": ["embedding"], "model_info": {"bert.embedding_length": 384, "bert.context_length": 512}})
            } else {
                json!({"capabilities": ["completion"]})
            })
        }
    }
    Mock::given(method("POST")).and(path("/api/show")).respond_with(Show).mount(&local).await;
    let router = build_router(&router_config(&local, ""), keys(&["openai"]), None, false).unwrap();
    let (choices, problems) = zorix_providers::embedding_choices(&router, &CancellationToken::new()).await;
    let refs: Vec<String> = choices.iter().map(|c| c.reference()).collect();
    assert!(
        refs.contains(&"ollama/all-minilm:latest".to_string()) && !refs.iter().any(|r| r.contains("qwen3")),
        "{refs:?}"
    );
    assert!(refs.contains(&"openai/text-embedding-3-small".to_string()));
    let minilm = choices.iter().find(|c| c.id == "all-minilm:latest").unwrap();
    assert!((minilm.local, minilm.dimensions) == (true, Some(384)));
    assert!(choices.iter().filter(|c| c.provider == "openai").all(|c| !c.local));
    assert!(problems.is_empty(), "{problems:?}");
    // A server that is not running is reported, and the cloud models are still listed.
    let down = ProvidersConfig::parse(
        "[[provider]]\nname = \"ollama\"\nkind = \"ollama\"\nbase_url = \"http://127.0.0.1:9\"\n",
    )
    .unwrap();
    let router = build_router(&down, keys(&["openai"]), None, false).unwrap();
    let (choices, problems) = zorix_providers::embedding_choices(&router, &CancellationToken::new()).await;
    assert!(choices.iter().any(|c| c.provider == "openai"));
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].0, "ollama");
}
