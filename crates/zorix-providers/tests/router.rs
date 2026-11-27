//! The router with fake gateways: resolution, filtering, ordering, fallback, cooldown, local-only and describe.

use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use zorix_core::{
    ExecutionId, FinishReason, ManualClock, Message, ModelError, ModelErrorKind, ModelGateway, ModelProfile,
    ModelRequest, ModelResponse, StepPhase, TextSink, Timestamp, ToolDefinition, Usage,
};
use zorix_providers::catalog::{Catalog, Latency, ModelInfo, Price};
use zorix_providers::config::{ProviderConfig, ProviderKind, ProvidersConfig};
use zorix_providers::{ProviderEntry, Router};

/// A gateway that answers from a script of outcomes and remembers the models it was asked for.
struct Fake {
    name: &'static str,
    script: Mutex<Vec<Result<(), ModelError>>>,
    asked: Mutex<Vec<String>>,
}

impl Fake {
    fn new(name: &'static str, script: Vec<Result<(), ModelError>>) -> Arc<Self> {
        Arc::new(Self { name, script: Mutex::new(script), asked: Mutex::new(Vec::new()) })
    }

    fn always_ok(name: &'static str) -> Arc<Self> {
        Self::new(name, vec![])
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().clone()
    }
}

#[async_trait]
impl ModelGateway for Fake {
    async fn generate(&self, request: ModelRequest, _cancel: CancellationToken) -> Result<ModelResponse, ModelError> {
        self.asked.lock().push(request.model.clone());
        let next = {
            let mut script = self.script.lock();
            if script.is_empty() { Ok(()) } else { script.remove(0) }
        };
        next?;
        Ok(ModelResponse::new(
            request.model,
            format!("from {}", self.name),
            vec![],
            Usage::default(),
            FinishReason::Stop,
        ))
    }

    fn describe(&self, _model: &str) -> Option<ModelProfile> {
        None
    }

    /// Starts writing before it knows whether it will fail, like a real stream.
    async fn generate_live(
        &self,
        request: ModelRequest,
        cancel: CancellationToken,
        sink: Arc<dyn TextSink>,
    ) -> Result<ModelResponse, ModelError> {
        sink.text(&format!("writing {}", self.name));
        self.generate(request, cancel).await
    }
}

struct Recorder(Mutex<Vec<String>>);

impl TextSink for Recorder {
    fn text(&self, delta: &str) {
        self.0.lock().push(delta.to_string());
    }

    fn reset(&self) {
        self.0.lock().push("<reset>".into());
    }
}

fn err(kind: ModelErrorKind) -> Result<(), ModelError> {
    Err(ModelError::provider(kind, format!("{} happened", kind.as_str())))
}

fn model(
    provider: &str,
    id: &str,
    window: u64,
    tools: bool,
    tier: u8,
    latency: Latency,
    out_price: Option<f64>,
) -> ModelInfo {
    ModelInfo {
        provider: provider.into(),
        id: id.into(),
        display_name: id.into(),
        context_window: window,
        max_output: 4_000.min(window / 2),
        tools,
        vision: false,
        thinking: false,
        latency,
        tier,
        price: out_price.map(|o| Price { input: o / 4.0, output: o, cache_read: None, cache_write: None }),
        assumed: false,
    }
}

fn entry(name: &str, kind: ProviderKind, url: Option<&str>, gateway: Arc<dyn ModelGateway>) -> ProviderEntry {
    let config =
        ProviderConfig { name: name.into(), kind, base_url: url.map(str::to_string), api_key_env: None, enabled: true };
    let local = config.is_local();
    ProviderEntry { config, gateway, local, discover: None }
}

fn catalog() -> Catalog {
    let mut c = Catalog::empty();
    for m in [
        model("big", "smart", 1_000_000, true, 5, Latency::Slow, Some(40.0)),
        model("big", "cheap", 1_000_000, true, 2, Latency::Fast, Some(1.0)),
        model("other", "plain", 1_000_000, false, 3, Latency::Standard, Some(5.0)),
        model("other", "tiny", 2_000, true, 1, Latency::Fast, None),
        model("home", "local", 32_000, true, 2, Latency::Standard, None),
    ] {
        c.insert(m).unwrap();
    }
    c
}

struct World {
    router: Router,
    big: Arc<Fake>,
    other: Arc<Fake>,
    home: Arc<Fake>,
    clock: Arc<ManualClock>,
}

fn world(config: &str, big: Vec<Result<(), ModelError>>) -> World {
    let (big, other, home) = (Fake::new("big", big), Fake::always_ok("other"), Fake::always_ok("home"));
    let clock = Arc::new(ManualClock::new(Timestamp(1_000_000)));
    let config = config_of(config);
    let router = Router::new(
        vec![
            entry("big", ProviderKind::Anthropic, Some("https://big.example.com"), big.clone()),
            entry("other", ProviderKind::OpenaiChat, Some("https://other.example.com/v1"), other.clone()),
            entry("home", ProviderKind::Ollama, Some("http://127.0.0.1:11434"), home.clone()),
        ],
        config,
        catalog(),
    )
    .with_clock(clock.clone());
    World { router, big, other, home, clock }
}

fn request(model: &str) -> ModelRequest {
    ModelRequest {
        model: model.into(),
        execution_id: ExecutionId::new(),
        messages: vec![Message::user("hi")],
        ..ModelRequest::default()
    }
}

fn with_tools(mut r: ModelRequest) -> ModelRequest {
    r.tools = vec![ToolDefinition { id: "t".into(), description: "d".into(), input_schema: json!({"type": "object"}) }];
    r
}

async fn ask(router: &Router, r: ModelRequest) -> Result<ModelResponse, ModelError> {
    router.generate(r, CancellationToken::new()).await
}

const PROVIDERS: &str = r#"
[[provider]]
name = "big"
kind = "anthropic"
base_url = "https://big.example.com"

[[provider]]
name = "other"
kind = "openai-chat"
base_url = "https://other.example.com/v1"

[[provider]]
name = "home"
kind = "ollama"
base_url = "http://127.0.0.1:11434"
"#;

fn config_of(text: &str) -> ProvidersConfig {
    ProvidersConfig::parse(&format!("{PROVIDERS}\n{text}")).unwrap()
}

const ROUTES: &str = r#"
[alias]
default = "route:main"
fast = "big/cheap"
home = "route:private"

[[route]]
name = "main"
candidates = ["big/smart", "other/plain", "home/local"]

[[route]]
name = "private"
candidates = ["big/smart", "home/local"]
local_only = true

[[route]]
name = "cheapest"
candidates = ["big/smart", "big/cheap", "home/local"]
order = "cheapest"

[[route]]
name = "best"
candidates = ["big/cheap", "other/plain", "big/smart"]
order = "best"

[[route]]
name = "quickest"
candidates = ["big/smart", "other/tiny", "big/cheap"]
order = "fastest"

[[route]]
name = "tiered"
candidates = ["big/cheap"]
start = ["big/smart"]
continue = ["home/local"]
"#;

#[tokio::test]
async fn an_explicit_model_goes_to_its_provider_with_the_bare_model_id_and_the_provider_name_is_recorded() {
    let w = world(ROUTES, vec![]);
    let response = ask(&w.router, request("big/smart")).await.unwrap();
    assert_eq!(
        (response.text.as_str(), response.provider.as_deref(), response.model.as_str()),
        ("from big", Some("big"), "smart")
    );
    assert_eq!(w.big.asked(), vec!["smart"]);
    let trace = response.route.unwrap();
    assert!(!trace.fell_back() && trace.attempts[0].outcome == "ok");
}

#[tokio::test]
async fn aliases_routes_and_the_builtin_names_resolve() {
    let w = world(ROUTES, vec![]);
    assert_eq!(ask(&w.router, request("fast")).await.unwrap().model, "cheap");
    assert_eq!(ask(&w.router, request("default")).await.unwrap().model, "smart");
    assert!(w.router.resolve("demo", StepPhase::Start).unwrap_err().contains("no provider called “sim”"));
    for bad in ["nobody/x", "route:missing", "unknown-alias", "UPPER", ""] {
        assert!(w.router.resolve(bad, StepPhase::Start).is_err(), "{bad:?}");
    }
    let error = ask(&w.router, request("nobody/x")).await.unwrap_err();
    assert_eq!(error.kind(), ModelErrorKind::Invalid);
    let no_default = Router::new(vec![], ProvidersConfig::default(), Catalog::empty());
    assert!(no_default.resolve("default", StepPhase::Start).unwrap_err().contains("No default model"));
}

#[tokio::test]
async fn a_route_falls_back_and_says_why_and_the_trace_lists_the_attempts() {
    let w = world(ROUTES, vec![err(ModelErrorKind::RateLimited)]);
    let response = ask(&w.router, request("route:main")).await.unwrap();
    assert_eq!((response.text.as_str(), response.provider.as_deref()), ("from other", Some("other")));
    let trace = response.route.unwrap();
    assert!(trace.fell_back());
    let outcomes: Vec<(String, String)> =
        trace.attempts.iter().map(|a| (format!("{}/{}", a.provider, a.model), a.outcome.clone())).collect();
    assert_eq!(outcomes, vec![("big/smart".into(), "rate_limited".into()), ("other/plain".into(), "ok".into())]);
}

#[tokio::test]
async fn every_fallback_worthy_error_moves_on_and_invalid_requests_do_not() {
    for kind in [
        ModelErrorKind::Auth,
        ModelErrorKind::NotFound,
        ModelErrorKind::ContextTooLong,
        ModelErrorKind::RateLimited,
        ModelErrorKind::Unavailable,
        ModelErrorKind::Refused,
    ] {
        let w = world(ROUTES, vec![err(kind)]);
        assert_eq!(ask(&w.router, request("route:main")).await.unwrap().provider.as_deref(), Some("other"), "{kind:?}");
    }
    let w = world(ROUTES, vec![err(ModelErrorKind::Invalid)]);
    let error = ask(&w.router, request("route:main")).await.unwrap_err();
    assert_eq!(error.kind(), ModelErrorKind::Invalid, "a request that cannot succeed is not sent elsewhere");
    assert!(w.other.asked().is_empty());
}

#[tokio::test]
async fn an_explicit_model_never_falls_back() {
    let w = world(ROUTES, vec![err(ModelErrorKind::Unavailable)]);
    let error = ask(&w.router, request("big/smart")).await.unwrap_err();
    assert_eq!(error.kind(), ModelErrorKind::Unavailable);
    assert!(w.other.asked().is_empty() && w.home.asked().is_empty());
}

#[tokio::test]
async fn when_everything_fails_the_error_lists_what_was_tried_and_keeps_the_last_kind() {
    let w = Router::new(
        vec![
            entry(
                "big",
                ProviderKind::Anthropic,
                Some("https://big.example.com"),
                Fake::new("big", vec![err(ModelErrorKind::RateLimited)]),
            ),
            entry(
                "other",
                ProviderKind::OpenaiChat,
                Some("https://other.example.com/v1"),
                Fake::new("other", vec![err(ModelErrorKind::Unavailable)]),
            ),
        ],
        config_of("[[route]]\nname = \"r\"\ncandidates = [\"big/smart\", \"other/tiny\"]\n"),
        catalog(),
    );
    let error = ask(&w, request("route:r")).await.unwrap_err();
    let text = error.to_string();
    assert_eq!(error.kind(), ModelErrorKind::Unavailable, "the engine retries a temporary kind");
    assert!(text.contains("big/smart (rate_limited)") && text.contains("other/tiny (unavailable)"), "{text}");
}

#[tokio::test]
async fn candidates_that_cannot_do_the_job_are_skipped_and_nothing_left_is_an_error_that_says_why() {
    let w = world(ROUTES, vec![]);
    // "other/plain" has no tool support: with tools offered it is skipped, so the order goes big → home.
    let r = ask(&w.router, with_tools(request("route:main"))).await.unwrap();
    assert_eq!(r.provider.as_deref(), Some("big"));
    let broken = world(ROUTES, vec![err(ModelErrorKind::Unavailable)]);
    let r = ask(&broken.router, with_tools(request("route:main"))).await.unwrap();
    assert_eq!(r.provider.as_deref(), Some("home"), "other/plain was skipped, not tried");
    assert!(broken.other.asked().is_empty());

    let only_plain = Router::new(
        vec![entry("other", ProviderKind::OpenaiChat, Some("https://other.example.com/v1"), Fake::always_ok("other"))],
        config_of("[[route]]\nname = \"r\"\ncandidates = [\"other/plain\"]\n"),
        catalog(),
    );
    let error = ask(&only_plain, with_tools(request("route:r"))).await.unwrap_err();
    assert_eq!(error.kind(), ModelErrorKind::Invalid);
    assert!(error.to_string().contains("other/plain: it cannot use tools"), "{error}");
}

#[tokio::test]
async fn a_conversation_too_long_for_a_candidate_skips_it() {
    let w = world("[[route]]\nname = \"r\"\ncandidates = [\"other/tiny\", \"big/cheap\"]\n", vec![]);
    let mut r = request("route:r");
    r.messages = vec![Message::user("word ".repeat(2_000))];
    assert_eq!(ask(&w.router, r).await.unwrap().model, "cheap", "the 2,000-token model could not hold it");
    assert!(w.other.asked().is_empty());
}

#[tokio::test]
async fn local_only_keeps_a_conversation_on_this_computer() {
    let w = world(ROUTES, vec![]);
    let r = ask(&w.router, request("route:private")).await.unwrap();
    assert_eq!((r.provider.as_deref(), w.big.asked().len()), (Some("home"), 0), "the route says local only");

    let strict = world(ROUTES, vec![]);
    let router = strict.router.local_only(true);
    assert_eq!(
        ask(&router, request("route:main")).await.unwrap().provider.as_deref(),
        Some("home"),
        "the setting overrides the route"
    );
    let error = ask(&router, request("big/smart")).await.unwrap_err();
    assert!(error.to_string().contains("local only"), "{error}");
    assert!(strict.big.asked().is_empty());
}

#[tokio::test]
async fn routes_can_order_by_price_quality_and_speed() {
    let first = |route: &str| {
        let w = world(ROUTES, vec![]);
        let name = route.to_string();
        async move { ask(&w.router, request(&format!("route:{name}"))).await.unwrap().model }
    };
    assert_eq!(first("cheapest").await, "cheap", "lowest output price first; unknown prices last");
    assert_eq!(first("best").await, "smart", "highest tier first");
    assert_eq!(first("quickest").await, "tiny", "fast before slow, and in listed order among equals");
}

#[tokio::test]
async fn phases_pick_different_models_for_different_steps() {
    let w = world(ROUTES, vec![]);
    let mut start = request("route:tiered");
    start.phase = StepPhase::Start;
    assert_eq!(ask(&w.router, start).await.unwrap().provider.as_deref(), Some("big"));
    let mut next = request("route:tiered");
    next.phase = StepPhase::Continue;
    assert_eq!(ask(&w.router, next).await.unwrap().provider.as_deref(), Some("home"));
    let mut retry = request("route:tiered");
    retry.phase = StepPhase::Retry;
    assert_eq!(ask(&w.router, retry).await.unwrap().model, "cheap", "no list for this phase: the route's own");
}

#[tokio::test]
async fn a_model_that_just_failed_rests_and_is_tried_again_after_the_cooldown() {
    let w = world(ROUTES, vec![err(ModelErrorKind::RateLimited)]);
    assert_eq!(ask(&w.router, request("route:main")).await.unwrap().provider.as_deref(), Some("other"));
    // Right after: big rests, so the first call goes to the next candidate without waiting for another failure.
    assert_eq!(ask(&w.router, request("route:main")).await.unwrap().provider.as_deref(), Some("other"));
    assert_eq!(w.big.asked().len(), 1, "big was not hammered");
    w.clock.advance(20_000);
    assert_eq!(ask(&w.router, request("route:main")).await.unwrap().provider.as_deref(), Some("big"), "rested enough");
}

#[tokio::test]
async fn when_everything_is_resting_the_router_still_tries() {
    let w = Router::new(
        vec![entry(
            "big",
            ProviderKind::Anthropic,
            Some("https://big.example.com"),
            Fake::new("big", vec![err(ModelErrorKind::RateLimited)]),
        )],
        config_of("[[route]]\nname = \"r\"\ncandidates = [\"big/smart\"]\n"),
        catalog(),
    );
    assert!(ask(&w, request("route:r")).await.is_err());
    assert!(ask(&w, request("route:r")).await.is_ok(), "the only candidate is resting, but it is still tried");
}

#[tokio::test]
async fn describe_gives_the_smallest_window_of_the_candidates_so_any_fallback_fits() {
    let w = world(ROUTES, vec![]);
    let profile = w.router.describe("route:main").unwrap();
    assert_eq!((profile.context_window, profile.max_output), (32_000, 4_000), "home/local is the smallest");
    assert!(
        !profile.tools,
        "one candidate of the route cannot use tools, so the route as a whole cannot be counted on"
    );
    assert!(w.router.describe("route:private").unwrap().tools);
    assert_eq!(w.router.describe("big/smart").unwrap().context_window, 1_000_000);
    // A call can be priced when the model has a price or runs on this computer; a remote model without one cannot (audit finding F11).
    assert!(w.router.describe("big/smart").unwrap().priced);
    assert!(w.router.describe("home/local").unwrap().priced, "a model on this computer costs nothing");
    assert!(!w.router.describe("other/tiny").unwrap().priced, "a remote model with no price is not priced");
    assert!(w.router.describe("route:main").unwrap().priced, "every candidate of the route has a price or runs here");
    let mixed = world("[[route]]\nname = \"mixed\"\ncandidates = [\"big/smart\", \"other/tiny\"]\n", vec![]);
    assert!(!mixed.router.describe("route:mixed").unwrap().priced, "one unpriced candidate makes the route unpriced");
    assert!(w.router.describe("big/unknown-model").is_none());
    assert!(w.router.describe("route:missing").is_none());
}

#[tokio::test]
async fn cancelling_stops_the_route() {
    let w = world(ROUTES, vec![]);
    let cancel = CancellationToken::new();
    cancel.cancel();
    let result = w.router.generate(request("route:main"), cancel).await;
    assert!(matches!(result, Err(ModelError::Cancelled)));
    assert!(w.big.asked().is_empty());
}

#[tokio::test]
async fn learning_a_discovered_model_makes_it_known() {
    let w = world(ROUTES, vec![]);
    assert!(w.router.describe("home/discovered").is_none());
    w.router.learn(vec![model("home", "discovered", 16_000, true, 2, Latency::Standard, None)]);
    assert_eq!(w.router.describe("home/discovered").unwrap().context_window, 16_000);
    assert!(w.router.known_models().iter().any(|m| m.id == "discovered"));
}

// ---------------------------------------------------------------------------------------------------------------------
// An agent's allow-list (ADR-0027)

fn allowing(mut r: ModelRequest, patterns: &[&str]) -> ModelRequest {
    r.allowed_models = patterns.iter().map(|p| p.to_string()).collect();
    r
}

#[tokio::test]
async fn a_fallback_never_reaches_a_model_the_agent_may_not_use() {
    // `big` fails; the route would move on to `other`, but the agent only allows `big`.
    let w = world(ROUTES, vec![err(ModelErrorKind::Unavailable)]);
    let error = ask(&w.router, allowing(request("route:main"), &["big/*"])).await.unwrap_err().to_string();
    assert!(error.contains("big/smart"), "{error}");
    assert!(w.other.asked().is_empty(), "the second candidate was never called");
    assert!(w.home.asked().is_empty());

    // With the second provider allowed, the same route falls back as before.
    let w = world(ROUTES, vec![err(ModelErrorKind::Unavailable)]);
    let ok = ask(&w.router, allowing(request("route:main"), &["big/*", "other/*"])).await.unwrap();
    assert_eq!(ok.provider.as_deref(), Some("other"));
    assert!(w.home.asked().is_empty(), "and the third was never needed");
}

#[tokio::test]
async fn candidates_outside_the_list_are_left_out_with_a_reason_and_an_empty_result_says_so() {
    let w = world(ROUTES, vec![]);
    let error = ask(&w.router, allowing(request("route:main"), &["nobody/*"])).await.unwrap_err().to_string();
    assert!(error.contains("this agent may not use it"), "{error}");
    assert!(w.big.asked().is_empty() && w.other.asked().is_empty() && w.home.asked().is_empty(), "nothing was sent");

    // An explicit model outside the list fails the same way, and one inside it goes through.
    let error = ask(&w.router, allowing(request("big/cheap"), &["other/*"])).await.unwrap_err().to_string();
    assert!(error.contains("this agent may not use it"), "{error}");
    assert!(ask(&w.router, allowing(request("big/cheap"), &["big/cheap"])).await.is_ok());
    assert!(
        ask(&w.router, allowing(request("big/cheap"), &["big/smart"])).await.is_err(),
        "an exact model, not its neighbour"
    );
}

#[tokio::test]
async fn an_empty_or_star_list_allows_everything_and_the_demo_models_can_be_allowed_by_name() {
    let w = world(ROUTES, vec![]);
    assert!(ask(&w.router, request("big/smart")).await.is_ok(), "no list, no restriction");
    assert!(ask(&w.router, allowing(request("big/smart"), &["*"])).await.is_ok());
    // Ordering still applies inside the allowed set.
    let cheapest = ask(&w.router, allowing(request("route:cheapest"), &["big/*"])).await.unwrap();
    assert_eq!(cheapest.model, "cheap");
}

#[tokio::test]
async fn live_text_starts_again_when_a_fallback_takes_over() {
    let w = world(ROUTES, vec![err(ModelErrorKind::RateLimited)]);
    let recorder = Arc::new(Recorder(Mutex::new(Vec::new())));
    let response =
        w.router.generate_live(request("route:main"), CancellationToken::new(), recorder.clone()).await.unwrap();
    assert_eq!(response.text, "from other");
    assert_eq!(*recorder.0.lock(), ["writing big", "<reset>", "writing other"]);

    // A first try that works is not reset.
    let recorder = Arc::new(Recorder(Mutex::new(Vec::new())));
    w.router.generate_live(request("route:main"), CancellationToken::new(), recorder.clone()).await.unwrap();
    assert_eq!(
        recorder.0.lock().first().map(String::as_str),
        Some("writing other"),
        "big is resting after its failure"
    );
    assert!(!recorder.0.lock().contains(&"<reset>".to_string()));
}
