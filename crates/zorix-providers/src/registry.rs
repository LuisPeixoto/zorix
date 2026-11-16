//! Building the router from `providers.toml`, asking providers what they offer, and reporting their state.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use zorix_core::{ModelGateway, Secret};

use crate::adapter::Discover;
use crate::anthropic::{AnthropicGateway, AnthropicOptions};
use crate::catalog::{Catalog, ModelInfo};
use crate::config::{ProviderConfig, ProviderKind, ProvidersConfig};
use crate::gemini::{GeminiGateway, GeminiOptions};
use crate::http::{HttpClient, HttpConfig, validate_base_url};
use crate::ollama::{OllamaGateway, OllamaOptions};
use crate::openai_chat::{OpenAiChatGateway, OpenAiChatOptions};
use crate::openai_responses::{OpenAiOptions, OpenAiResponsesGateway};
use crate::router::{ProviderEntry, Router};
use crate::secrets::{ChainStore, EnvStore, KeychainStore, SecretStore};

/// The model the default is built around when Anthropic has a key.
pub const DEFAULT_ANTHROPIC_MODEL: &str = "anthropic/claude-opus-5-5";

/// The secret store the apps use: environment variables (including each provider's `api_key_env`), then the keychain.
pub fn standard_secrets(config: &ProvidersConfig) -> Arc<dyn SecretStore> {
    secrets_with(config, true)
}

/// The same, with the keychain left out when `keychain` is false (servers, CI and tests: `ZORIX_SECRETS=env`).
pub fn secrets_with(config: &ProvidersConfig, keychain: bool) -> Arc<dyn SecretStore> {
    let env = config
        .instances()
        .iter()
        .filter_map(|p| p.api_key_env.as_ref().map(|var| (p.name.clone(), var.clone())))
        .fold(EnvStore::new(), |env, (name, var)| env.with_variable(&name, &var));
    let mut stores: Vec<Arc<dyn SecretStore>> = vec![Arc::new(env)];
    if keychain {
        stores.push(Arc::new(KeychainStore::new()));
    }
    Arc::new(ChainStore::new(stores))
}

/// The catalogue: what ships with ZORIX plus the models in the configuration.
pub fn catalog_for(config: &ProvidersConfig) -> Result<Catalog, String> {
    let mut catalog = Catalog::embedded();
    for model in &config.models {
        catalog.insert(model.clone())?;
    }
    Ok(catalog)
}

/// Build the router: one gateway per enabled provider instance, plus the simulated models when `sim` is given.
pub fn build_router(
    config: &ProvidersConfig,
    secrets: Arc<dyn SecretStore>,
    sim: Option<Arc<dyn ModelGateway>>,
    local_only: bool,
) -> Result<Router, String> {
    config.validate()?;
    let catalog = Arc::new(catalog_for(config)?);
    let mut entries: Vec<ProviderEntry> = Vec::new();
    for instance in config.instances().into_iter().filter(|p| p.enabled) {
        let url = validate_base_url(instance.url()).map_err(|e| format!("Provider “{}”: {e}", instance.name))?;
        let http = HttpClient::new(&instance.name, HttpConfig::default())?;
        let name = instance.name.as_str();
        let (gateway, discover): (Arc<dyn ModelGateway>, Option<Arc<dyn Discover>>) = match instance.kind {
            ProviderKind::Anthropic => {
                let g = Arc::new(AnthropicGateway::new(
                    name,
                    http,
                    &url,
                    secrets.clone(),
                    name,
                    catalog.clone(),
                    AnthropicOptions::default(),
                )?);
                (g.clone(), Some(g))
            }
            ProviderKind::Openai => {
                let g = Arc::new(OpenAiResponsesGateway::new(
                    name,
                    http,
                    &url,
                    secrets.clone(),
                    name,
                    catalog.clone(),
                    OpenAiOptions::default(),
                )?);
                (g.clone(), Some(g))
            }
            ProviderKind::OpenaiChat => {
                let g = Arc::new(OpenAiChatGateway::new(
                    name,
                    http,
                    &url,
                    secrets.clone(),
                    name,
                    catalog.clone(),
                    OpenAiChatOptions::default(),
                )?);
                (g.clone(), Some(g))
            }
            ProviderKind::Gemini => {
                let g = Arc::new(GeminiGateway::new(
                    name,
                    http,
                    &url,
                    secrets.clone(),
                    name,
                    catalog.clone(),
                    GeminiOptions::default(),
                ));
                (g.clone(), Some(g))
            }
            ProviderKind::Ollama => {
                let g = Arc::new(OllamaGateway::new(name, http, &url, catalog.clone(), OllamaOptions::default()));
                (g.clone(), Some(g))
            }
            ProviderKind::Sim => continue,
        };
        let local = instance.is_local();
        entries.push(ProviderEntry { config: instance, gateway, local, discover });
    }
    if let Some(sim) = sim {
        entries.push(ProviderEntry {
            config: ProviderConfig::builtin(ProviderKind::Sim),
            gateway: sim,
            local: true,
            discover: None,
        });
    }
    let mut router = Router::new(entries, config.clone(), (*catalog).clone()).local_only(local_only);
    if let Some(default) = default_for(config, &secrets) {
        router = router.with_default(&default);
    }
    Ok(router)
}

/// The default model when the configuration does not set one: Anthropic's, if there is a key for it.
fn default_for(config: &ProvidersConfig, secrets: &Arc<dyn SecretStore>) -> Option<String> {
    if config.alias.contains_key("default") {
        return None; // the router resolves the alias itself
    }
    let has_key = |name: &str| matches!(secrets.get(name), Ok(Some(_)));
    let anthropic_on = config.instances().iter().any(|p| p.name == "anthropic" && p.enabled);
    (anthropic_on && has_key("anthropic")).then(|| DEFAULT_ANTHROPIC_MODEL.to_string())
}

/// Whether a provider has what it needs to be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyStatus {
    /// It needs no key (a local server).
    NotNeeded,
    /// A key is stored (or set in the environment).
    Present,
    /// It needs a key and there is none.
    Missing,
    /// The keychain could not be read, and why.
    Unavailable(String),
}

/// One line of `zorix provider list`.
#[derive(Clone, Debug)]
pub struct ProviderStatus {
    /// The instance name.
    pub name: String,
    /// Its kind.
    pub kind: ProviderKind,
    /// Its address.
    pub url: String,
    /// On this computer.
    pub local: bool,
    /// Turned on.
    pub enabled: bool,
    /// Its key.
    pub key: KeyStatus,
}

/// The state of every provider instance in the configuration.
pub fn provider_statuses(config: &ProvidersConfig, secrets: &Arc<dyn SecretStore>) -> Vec<ProviderStatus> {
    config
        .instances()
        .into_iter()
        .map(|p| {
            let key = if !p.kind.needs_key() && !matches!(secrets.get(&p.name), Ok(Some(_))) {
                KeyStatus::NotNeeded
            } else {
                match secrets.get(&p.name) {
                    Ok(Some(_)) => KeyStatus::Present,
                    Ok(None) if p.kind.needs_key() && !p.is_local() => KeyStatus::Missing,
                    Ok(None) => KeyStatus::NotNeeded,
                    Err(e) => KeyStatus::Unavailable(e.to_string()),
                }
            };
            ProviderStatus {
                name: p.name.clone(),
                kind: p.kind,
                url: p.url().to_string(),
                local: p.is_local(),
                enabled: p.enabled,
                key,
            }
        })
        .collect()
}

/// What discovery found at one provider.
#[derive(Clone, Debug)]
pub struct Discovered {
    /// The provider instance.
    pub provider: String,
    /// The models, or why they could not be listed.
    pub models: Result<Vec<ModelInfo>, String>,
}

/// Ask every provider that has what it needs (a key, or a local address) which models it offers, and teach the router.
pub async fn discover_all(router: &Router, cancel: &CancellationToken) -> Vec<Discovered> {
    let mut out = Vec::new();
    let mut names: Vec<&str> = router.providers().map(|p| p.config.name.as_str()).collect();
    names.sort_unstable();
    for name in names {
        let Some(entry) = router.provider(name) else { continue };
        let Some(discover) = &entry.discover else { continue };
        if !entry.config.enabled {
            continue;
        }
        let result = discover.discover(cancel).await.map_err(|e| e.to_string());
        if let Ok(models) = &result {
            router.learn(models.clone());
        }
        out.push(Discovered { provider: name.to_string(), models: result });
    }
    out
}

/// A name for a key for display, never the key itself.
pub fn describe_secret(secret: &Secret) -> String {
    format!("{} characters", secret.expose().chars().count())
}
