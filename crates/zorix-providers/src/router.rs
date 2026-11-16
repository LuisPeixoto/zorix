//! The router: turns "which model?" into an ordered list of candidates, tries them, and falls back (ADR-0010).
//!
//! It is itself a `ModelGateway`, so the runtime sees one model. The request's `model` is what a person typed:
//! `provider/model`, a short alias, or `route:<name>`.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use zorix_core::{
    Clock, Message, ModelError, ModelErrorKind, ModelGateway, ModelProfile, ModelRequest, ModelResponse, RouteAttempt,
    RouteTrace, StepPhase, SystemClock, Timestamp,
};

use crate::catalog::{Catalog, Latency, ModelInfo};
use crate::config::{Capability, Order, ProviderConfig, ProvidersConfig, Selector};

/// One provider instance the router can use.
#[derive(Clone)]
pub struct ProviderEntry {
    /// Its configuration.
    pub config: ProviderConfig,
    /// The gateway that talks to it.
    pub gateway: Arc<dyn ModelGateway>,
    /// True when it runs on this computer.
    pub local: bool,
    /// How to ask it which models it offers, when it can say.
    pub discover: Option<Arc<dyn crate::adapter::Discover>>,
}

/// A candidate: a provider instance and one of its models.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// The provider instance.
    pub provider: String,
    /// The model id.
    pub model: String,
}

impl Candidate {
    fn label(&self) -> String {
        format!("{}/{}", self.provider, self.model)
    }
}

/// What a selector resolved to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// The candidates, before filtering and ordering.
    pub candidates: Vec<Candidate>,
    /// Required capabilities.
    pub require: Vec<Capability>,
    /// How to order the ones that qualify.
    pub order: Order,
    /// Only providers on this computer.
    pub local_only: bool,
    /// True when a person named exactly one model: no fallback to another model.
    pub explicit: bool,
}

/// The routing table and the gateways behind it.
pub struct Router {
    providers: HashMap<String, ProviderEntry>,
    config: ProvidersConfig,
    catalog: RwLock<Catalog>,
    default_selector: Option<String>,
    local_only: bool,
    cooldowns: Mutex<HashMap<(String, String), Timestamp>>,
    clock: Arc<dyn Clock>,
}

impl Router {
    /// A router over these providers.
    pub fn new(providers: Vec<ProviderEntry>, config: ProvidersConfig, catalog: Catalog) -> Self {
        Self {
            providers: providers.into_iter().map(|p| (p.config.name.clone(), p)).collect(),
            config,
            catalog: RwLock::new(catalog),
            default_selector: None,
            local_only: false,
            cooldowns: Mutex::new(HashMap::new()),
            clock: Arc::new(SystemClock),
        }
    }

    /// What `default` means when the configuration does not say.
    pub fn with_default(mut self, selector: &str) -> Self {
        self.default_selector = Some(selector.to_string());
        self
    }

    /// Only use providers on this computer, whatever the routes say.
    pub fn local_only(mut self, on: bool) -> Self {
        self.local_only = on;
        self
    }

    /// Use another clock (tests control the cooldowns with it).
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// The provider instances, by name.
    pub fn providers(&self) -> impl Iterator<Item = &ProviderEntry> {
        self.providers.values()
    }

    /// One provider instance.
    pub fn provider(&self, name: &str) -> Option<&ProviderEntry> {
        self.providers.get(name)
    }

    /// The configuration in use.
    pub fn config(&self) -> &ProvidersConfig {
        &self.config
    }

    /// Learn about models found by discovery.
    pub fn learn(&self, models: Vec<ModelInfo>) {
        let mut catalog = self.catalog.write();
        for model in models {
            let _ = catalog.insert(model);
        }
    }

    /// What is known about a model.
    pub fn model_info(&self, provider: &str, model: &str) -> Option<ModelInfo> {
        self.catalog.read().get(provider, model).cloned()
    }

    /// Every model known so far.
    pub fn known_models(&self) -> Vec<ModelInfo> {
        self.catalog.read().list().cloned().collect()
    }

    /// Turn what a person typed into candidates. `phase` picks the route's list for that phase, when it has one.
    pub fn resolve(&self, selector: &str, phase: StepPhase) -> Result<Resolved, String> {
        self.resolve_inner(selector, phase, 0)
    }

    fn resolve_inner(&self, selector: &str, phase: StepPhase, depth: u8) -> Result<Resolved, String> {
        if depth > 2 {
            return Err(format!("“{selector}” points around in a circle."));
        }
        match Selector::parse(selector)? {
            Selector::Model { provider, model } => {
                if !self.providers.contains_key(&provider) {
                    let known: Vec<&str> = self.providers.keys().map(String::as_str).collect();
                    return Err(format!("There is no provider called “{provider}”. Known: {}.", known.join(", ")));
                }
                Ok(Resolved {
                    candidates: vec![Candidate { provider, model }],
                    require: Vec::new(),
                    order: Order::Listed,
                    local_only: false,
                    explicit: true,
                })
            }
            Selector::Route(name) => {
                let route = self.config.route(&name).ok_or_else(|| format!("There is no route called “{name}”."))?;
                let list = match phase {
                    StepPhase::Start => route.start.as_ref(),
                    StepPhase::Continue => route.continue_.as_ref(),
                    StepPhase::Retry => route.retry.as_ref(),
                }
                .unwrap_or(&route.candidates);
                let mut candidates = Vec::new();
                for entry in list {
                    match Selector::parse(entry)? {
                        Selector::Model { provider, model } => candidates.push(Candidate { provider, model }),
                        _ => return Err(format!("Route “{name}” lists “{entry}”, which is not a model.")),
                    }
                }
                Ok(Resolved {
                    candidates,
                    require: route.require.clone(),
                    order: route.order,
                    local_only: route.local_only,
                    explicit: false,
                })
            }
            Selector::Alias(name) => {
                if let Some(target) = self.config.alias.get(&name) {
                    return self.resolve_inner(target, phase, depth + 1);
                }
                match name.as_str() {
                    "demo" => self.resolve_inner("sim/demo", phase, depth + 1),
                    "script" => self.resolve_inner("sim/script", phase, depth + 1),
                    "default" => match &self.default_selector {
                        Some(target) => self.resolve_inner(target, phase, depth + 1),
                        None => Err("No default model is set. Name one with --model provider/model, or add `default = \"…\"` under [alias] in providers.toml.".into()),
                    },
                    other => Err(format!("“{other}” is not a model, an alias or a route. Try provider/model, such as anthropic/claude-opus-5-5.")),
                }
            }
        }
    }

    fn estimate_tokens(request: &ModelRequest) -> u64 {
        let messages: u64 = request.messages.iter().map(Message::estimated_tokens).sum();
        let tools: u64 =
            request.tools.iter().map(|t| (t.description.len() + t.input_schema.to_string().len()) as u64 / 4 + 8).sum();
        messages + tools
    }

    /// Filter and order the candidates for one request. Returns the ones to try and, for the ones left out, why.
    fn plan(&self, resolved: &Resolved, request: &ModelRequest) -> (Vec<Candidate>, Vec<String>) {
        let local_only = self.local_only || self.config.local_only || resolved.local_only;
        let mut require = resolved.require.clone();
        if !request.tools.is_empty() && !require.contains(&Capability::Tools) {
            require.push(Capability::Tools);
        }
        let needed = Self::estimate_tokens(request);
        let mut kept: Vec<(Candidate, Option<ModelInfo>)> = Vec::new();
        let mut left_out = Vec::new();
        for candidate in &resolved.candidates {
            let Some(entry) = self.providers.get(&candidate.provider) else {
                left_out.push(format!("{}: no such provider", candidate.label()));
                continue;
            };
            if !entry.config.enabled {
                left_out.push(format!("{}: the provider is turned off", candidate.label()));
                continue;
            }
            if local_only && !entry.local {
                left_out.push(format!("{}: it is not on this computer (local only)", candidate.label()));
                continue;
            }
            let info = self.model_info(&candidate.provider, &candidate.model);
            if let Some(info) = &info {
                if require.contains(&Capability::Tools) && !info.tools {
                    left_out.push(format!("{}: it cannot use tools", candidate.label()));
                    continue;
                }
                if require.contains(&Capability::Vision) && !info.vision {
                    left_out.push(format!("{}: it cannot read images", candidate.label()));
                    continue;
                }
                let room = info.context_window.saturating_sub(info.max_output.min(info.context_window / 2));
                if needed > room {
                    left_out.push(format!("{}: the conversation is too long for it", candidate.label()));
                    continue;
                }
            }
            kept.push((candidate.clone(), info));
        }
        match resolved.order {
            Order::Listed => {}
            Order::Cheapest => kept.sort_by(|a, b| price_key(&a.1).total_cmp(&price_key(&b.1))),
            Order::Fastest => kept.sort_by_key(|(_, info)| info.as_ref().map(|i| i.latency).unwrap_or(Latency::Slow)),
            Order::Best => kept.sort_by_key(|(_, info)| std::cmp::Reverse(info.as_ref().map(|i| i.tier).unwrap_or(0))),
        }
        (kept.into_iter().map(|(c, _)| c).collect(), left_out)
    }

    fn cooling(&self, candidate: &Candidate) -> bool {
        let now = self.clock.now();
        self.cooldowns
            .lock()
            .get(&(candidate.provider.clone(), candidate.model.clone()))
            .is_some_and(|until| *until > now)
    }

    fn cool_down(&self, candidate: &Candidate, error: &ModelError) {
        let wait_ms = match error {
            ModelError::Provider { kind: ModelErrorKind::RateLimited, retry_after_ms, .. } => {
                retry_after_ms.unwrap_or(0).max(15_000)
            }
            ModelError::Provider { kind: ModelErrorKind::Unavailable, .. } | ModelError::Transient { .. } => 5_000,
            ModelError::Provider { kind: ModelErrorKind::Auth, .. } => 10_000,
            _ => return,
        };
        let until = self.clock.now().plus_millis(wait_ms);
        self.cooldowns.lock().insert((candidate.provider.clone(), candidate.model.clone()), until);
    }
}

fn price_key(info: &Option<ModelInfo>) -> f64 {
    info.as_ref().and_then(|i| i.price).map(|p| p.output).unwrap_or(f64::MAX)
}

#[async_trait]
impl ModelGateway for Router {
    async fn generate(&self, request: ModelRequest, cancel: CancellationToken) -> Result<ModelResponse, ModelError> {
        let resolved = self
            .resolve(&request.model, request.phase)
            .map_err(|e| ModelError::provider(ModelErrorKind::Invalid, e))?;
        let (candidates, left_out) = self.plan(&resolved, &request);
        if candidates.is_empty() {
            let why = if left_out.is_empty() { "nothing was listed".to_string() } else { left_out.join("; ") };
            return Err(ModelError::provider(
                ModelErrorKind::Invalid,
                format!("No model can answer this request: {why}."),
            ));
        }
        // Candidates that rested after a recent failure go last: tried only if everything else fails.
        let (ready, resting): (Vec<_>, Vec<_>) = candidates.into_iter().partition(|c| !self.cooling(c));
        let mut attempts: Vec<RouteAttempt> = Vec::new();
        let mut last_error: Option<ModelError> = None;
        for candidate in ready.into_iter().chain(resting) {
            if cancel.is_cancelled() {
                return Err(ModelError::Cancelled);
            }
            let Some(entry) = self.providers.get(&candidate.provider) else { continue };
            let mut attempt_request = request.clone();
            attempt_request.model = candidate.model.clone();
            match entry.gateway.generate(attempt_request, cancel.clone()).await {
                Ok(mut response) => {
                    attempts.push(RouteAttempt {
                        provider: candidate.provider.clone(),
                        model: candidate.model.clone(),
                        outcome: "ok".into(),
                    });
                    response.provider = Some(candidate.provider.clone());
                    response.route = Some(RouteTrace { attempts });
                    return Ok(response);
                }
                Err(error) => {
                    let kind = error.kind();
                    attempts.push(RouteAttempt {
                        provider: candidate.provider.clone(),
                        model: candidate.model.clone(),
                        outcome: kind.as_str().into(),
                    });
                    if kind == ModelErrorKind::Cancelled {
                        return Err(error);
                    }
                    self.cool_down(&candidate, &error);
                    if !kind.allows_fallback() || resolved.explicit {
                        return Err(error);
                    }
                    last_error = Some(error);
                }
            }
        }
        let tried: Vec<String> =
            attempts.iter().map(|a| format!("{}/{} ({})", a.provider, a.model, a.outcome)).collect();
        let kind = last_error.as_ref().map(ModelError::kind).unwrap_or(ModelErrorKind::Unavailable);
        let retry_after_ms = match &last_error {
            Some(ModelError::Provider { retry_after_ms, .. }) => *retry_after_ms,
            _ => None,
        };
        let detail = last_error.map(|e| format!(" Last error: {e}")).unwrap_or_default();
        Err(ModelError::Provider {
            kind,
            message: format!("No model could answer. Tried: {}.{detail}", tried.join(", ")),
            retry_after_ms,
        })
    }

    fn describe(&self, model: &str) -> Option<ModelProfile> {
        let resolved = self.resolve(model, StepPhase::Start).ok()?;
        let profiles: Vec<ModelProfile> = resolved
            .candidates
            .iter()
            .filter_map(|c| {
                self.model_info(&c.provider, &c.model)
                    .map(|i| i.profile())
                    .or_else(|| self.providers.get(&c.provider)?.gateway.describe(&c.model))
            })
            .collect();
        // The smallest window and output limit: whichever candidate answers, the conversation fits.
        let known = profiles.len() == resolved.candidates.len() && !profiles.is_empty();
        known.then(|| ModelProfile {
            context_window: profiles.iter().map(|p| p.context_window).min().unwrap_or(0),
            max_output: profiles.iter().map(|p| p.max_output).min().unwrap_or(0),
            tools: profiles.iter().all(|p| p.tools),
            vision: profiles.iter().all(|p| p.vision),
        })
    }
}
