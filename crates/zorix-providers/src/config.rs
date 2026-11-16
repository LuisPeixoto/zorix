//! `providers.toml`: which providers exist, how models are named and which routes there are.
//!
//! The file is optional: with none, ZORIX offers each built-in provider that has a key (or, for Ollama, answers). It never
//! holds a key: a key in the file is refused with a message that says where keys go (ADR-0011).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::catalog::ModelInfo;
use crate::http::validate_base_url;

/// The file name inside the data folder.
pub const FILE_NAME: &str = "providers.toml";

/// Where the file lives for a data folder.
pub fn config_path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}

/// Which API family a provider speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    /// Anthropic's Messages API.
    Anthropic,
    /// OpenAI's Responses API.
    Openai,
    /// A Chat Completions server (OpenRouter, Groq, vLLM, LM Studio, or OpenAI itself for older models).
    OpenaiChat,
    /// Google's `generateContent` API.
    Gemini,
    /// Ollama's native API, with discovery.
    Ollama,
    /// The built-in simulated models. Reserved: it cannot be configured.
    Sim,
}

impl ProviderKind {
    /// Where the provider is when no address is configured.
    pub fn default_base_url(&self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "https://api.anthropic.com",
            ProviderKind::Openai => "https://api.openai.com/v1",
            ProviderKind::OpenaiChat => "https://api.openai.com/v1",
            ProviderKind::Gemini => "https://generativelanguage.googleapis.com",
            ProviderKind::Ollama => "http://127.0.0.1:11434",
            ProviderKind::Sim => "http://127.0.0.1",
        }
    }

    /// True when the provider normally needs an API key.
    pub fn needs_key(&self) -> bool {
        !matches!(self, ProviderKind::Ollama | ProviderKind::Sim)
    }

    /// The name used in the file and in messages.
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::Openai => "openai",
            ProviderKind::OpenaiChat => "openai-chat",
            ProviderKind::Gemini => "gemini",
            ProviderKind::Ollama => "ollama",
            ProviderKind::Sim => "sim",
        }
    }
}

fn yes() -> bool {
    true
}

/// Well-known servers that speak the Chat Completions API: `(name, address, needs a key)`.
pub const CHAT_PRESETS: &[(&str, &str, bool)] = &[
    ("openrouter", "https://openrouter.ai/api/v1", true),
    ("groq", "https://api.groq.com/openai/v1", true),
    ("together", "https://api.together.xyz/v1", true),
    ("lmstudio", "http://127.0.0.1:1234/v1", false),
    ("vllm", "http://127.0.0.1:8000/v1", false),
];

/// One provider instance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    /// The instance name: the prefix of model references and the name of its key.
    pub name: String,
    /// Which API family it speaks.
    pub kind: ProviderKind,
    /// Its address. Defaults to the kind's usual address. HTTPS, or HTTP to a loopback address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// An environment variable that may hold its key, besides the usual ones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    /// Turn it off without deleting it.
    #[serde(default = "yes")]
    pub enabled: bool,
}

impl ProviderConfig {
    /// An instance of a kind with defaults, named after the kind.
    pub fn builtin(kind: ProviderKind) -> Self {
        Self { name: kind.as_str().to_string(), kind, base_url: None, api_key_env: None, enabled: true }
    }

    /// The address in use.
    pub fn url(&self) -> &str {
        self.base_url.as_deref().unwrap_or_else(|| self.kind.default_base_url())
    }

    /// True when the provider's address is on this computer.
    pub fn is_local(&self) -> bool {
        validate_base_url(self.url()).ok().and_then(|u| u.host_str().map(crate::http::is_loopback)).unwrap_or(false)
    }
}

/// What a route needs a model to be able to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Ask for tools.
    Tools,
    /// Read images.
    Vision,
}

/// How a route orders the candidates that qualify.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Order {
    /// As listed.
    #[default]
    Listed,
    /// Cheapest output price first (models with no known price last).
    Cheapest,
    /// Fastest latency class first.
    Fastest,
    /// Highest quality tier first.
    Best,
}

/// A named policy for choosing among models.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteConfig {
    /// The route's name, used as `route:<name>`.
    pub name: String,
    /// The models to consider, as `provider/model`.
    #[serde(default)]
    pub candidates: Vec<String>,
    /// Candidates that cannot do these things are skipped.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub require: Vec<Capability>,
    /// How to order the candidates that qualify.
    #[serde(default)]
    pub order: Order,
    /// Only use providers on this computer.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub local_only: bool,
    /// Candidates for the first call of an execution, when different from `candidates`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<Vec<String>>,
    /// Candidates for calls after tool results.
    #[serde(default, rename = "continue", skip_serializing_if = "Option::is_none")]
    pub continue_: Option<Vec<String>>,
    /// Candidates for a call repeated after a rejected answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<Vec<String>>,
}

/// The whole file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvidersConfig {
    /// Only use providers on this computer, for every route.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub local_only: bool,
    /// Provider instances (besides the built-in ones, which can be overridden by name).
    #[serde(default, rename = "provider", skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<ProviderConfig>,
    /// Models to add to the catalogue or whose numbers to override.
    #[serde(default, rename = "model", skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<ModelInfo>,
    /// Short names for models: `fast = "anthropic/claude-haiku-5-5"`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub alias: BTreeMap<String, String>,
    /// Routes.
    #[serde(default, rename = "route", skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<RouteConfig>,
}

/// A model as a person writes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selector {
    /// `provider/model`. The model may itself contain slashes (`openrouter/openai/some-model`).
    Model {
        /// The provider instance.
        provider: String,
        /// The model id.
        model: String,
    },
    /// `route:<name>`.
    Route(String),
    /// A short name from `[alias]`.
    Alias(String),
}

impl Selector {
    /// Parse what a person typed.
    pub fn parse(text: &str) -> Result<Selector, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("Name a model, such as anthropic/claude-opus-5-5.".into());
        }
        if let Some(name) = text.strip_prefix("route:") {
            return slug(name, "route").map(|()| Selector::Route(name.to_string()));
        }
        match text.split_once('/') {
            Some((provider, model)) if !model.is_empty() => {
                slug(provider, "provider")?;
                Ok(Selector::Model { provider: provider.to_string(), model: model.to_string() })
            }
            Some(_) => Err(format!("“{text}” has no model after the slash.")),
            None => slug(text, "alias").map(|()| Selector::Alias(text.to_string())),
        }
    }
}

fn slug(text: &str, what: &str) -> Result<(), String> {
    let ok = !text.is_empty()
        && text.len() <= 64
        && text.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'));
    if ok {
        Ok(())
    } else {
        Err(format!("“{text}” is not a valid {what} name: use lowercase letters, digits, '-', '_' or '.'."))
    }
}

const KEY_LIKE: &[&str] = &["api_key", "apikey", "api-key", "key", "token", "password", "secret", "authorization"];

fn find_key_like(value: &toml::Value, path: &str, found: &mut Vec<String>) {
    match value {
        toml::Value::Table(table) => {
            for (key, inner) in table {
                let here = if path.is_empty() { key.clone() } else { format!("{path}.{key}") };
                if KEY_LIKE.contains(&key.to_ascii_lowercase().as_str()) {
                    found.push(here.clone());
                }
                find_key_like(inner, &here, found);
            }
        }
        toml::Value::Array(items) => items.iter().for_each(|item| find_key_like(item, path, found)),
        _ => {}
    }
}

impl ProvidersConfig {
    /// Read the text of a configuration file.
    pub fn parse(text: &str) -> Result<Self, String> {
        let raw: toml::Value = toml::from_str(text).map_err(|e| format!("providers.toml is not valid: {e}"))?;
        let mut found = Vec::new();
        find_key_like(&raw, "", &mut found);
        if !found.is_empty() {
            return Err(format!(
                "providers.toml must not hold keys (found {}). Keep keys in the keychain with `zorix provider set-key <name>`, or in an environment variable named by `api_key_env`.",
                found.join(", ")
            ));
        }
        let config: ProvidersConfig = toml::from_str(text).map_err(|e| format!("providers.toml is not valid: {e}"))?;
        config.validate()?;
        Ok(config)
    }

    /// Read the file at `path`. A missing file is an empty configuration.
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("Cannot read {}: {e}", path.display())),
        }
    }

    /// Write the file atomically (a temporary file, then a rename), readable only by the owner on Unix.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        self.validate()?;
        let text = toml::to_string_pretty(self).map_err(|e| format!("Cannot write the configuration: {e}"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("Cannot create {}: {e}", parent.display()))?;
        }
        let temporary = path.with_extension("toml.tmp");
        std::fs::write(&temporary, text).map_err(|e| format!("Cannot write {}: {e}", temporary.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&temporary, path).map_err(|e| {
            let _ = std::fs::remove_file(&temporary);
            format!("Cannot write {}: {e}", path.display())
        })
    }

    /// Every provider instance: the built-in ones, replaced by name by the configured ones, then the extra ones.
    pub fn instances(&self) -> Vec<ProviderConfig> {
        let mut all: Vec<ProviderConfig> =
            [ProviderKind::Anthropic, ProviderKind::Openai, ProviderKind::Gemini, ProviderKind::Ollama]
                .into_iter()
                .map(ProviderConfig::builtin)
                .collect();
        for configured in &self.providers {
            match all.iter_mut().find(|p| p.name == configured.name) {
                Some(existing) => *existing = configured.clone(),
                None => all.push(configured.clone()),
            }
        }
        all
    }

    /// The route called `name`.
    pub fn route(&self, name: &str) -> Option<&RouteConfig> {
        self.routes.iter().find(|r| r.name == name)
    }

    /// Check everything that can be checked without talking to a provider. Returns the first problem, in words.
    pub fn validate(&self) -> Result<(), String> {
        let mut names = BTreeSet::new();
        for provider in &self.providers {
            slug(&provider.name, "provider")?;
            if provider.name == "sim" || provider.kind == ProviderKind::Sim {
                return Err("“sim” is reserved for the built-in simulated models.".into());
            }
            if !names.insert(provider.name.clone()) {
                return Err(format!("The provider “{}” is listed twice.", provider.name));
            }
            if let Some(url) = &provider.base_url {
                validate_base_url(url).map_err(|e| format!("Provider “{}”: {e}", provider.name))?;
            }
            if let Some(var) = &provider.api_key_env
                && (var.is_empty() || !var.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
            {
                return Err(format!(
                    "Provider “{}”: api_key_env must be an environment variable name such as MY_API_KEY.",
                    provider.name
                ));
            }
        }
        let known: BTreeSet<String> = self.instances().into_iter().map(|p| p.name).chain(["sim".to_string()]).collect();
        let check_ref = |text: &str, context: &str| -> Result<(), String> {
            match Selector::parse(text).map_err(|e| format!("{context}: {e}"))? {
                Selector::Model { provider, .. } if !known.contains(&provider) => {
                    Err(format!("{context}: there is no provider called “{provider}”."))
                }
                Selector::Model { .. } => Ok(()),
                Selector::Route(_) | Selector::Alias(_) => {
                    Err(format!("{context}: use a model such as anthropic/claude-opus-5-5, not “{text}”."))
                }
            }
        };
        let mut routes = BTreeSet::new();
        for route in &self.routes {
            slug(&route.name, "route")?;
            if !routes.insert(route.name.clone()) {
                return Err(format!("The route “{}” is listed twice.", route.name));
            }
            let context = format!("Route “{}”", route.name);
            let lists = [Some(&route.candidates), route.start.as_ref(), route.continue_.as_ref(), route.retry.as_ref()];
            if route.candidates.is_empty() {
                return Err(format!("{context} has no candidates."));
            }
            for list in lists.into_iter().flatten() {
                if list.is_empty() {
                    return Err(format!("{context} has an empty list of candidates."));
                }
                for candidate in list {
                    check_ref(candidate, &context)?;
                }
            }
        }
        for (alias, target) in &self.alias {
            slug(alias, "alias")?;
            let context = format!("Alias “{alias}”");
            match Selector::parse(target).map_err(|e| format!("{context}: {e}"))? {
                Selector::Route(name) if !routes.contains(&name) => {
                    return Err(format!("{context}: there is no route called “{name}”."));
                }
                Selector::Route(_) => {}
                Selector::Alias(_) => {
                    return Err(format!("{context} points at another alias; point it at a model or a route."));
                }
                Selector::Model { .. } => check_ref(target, &context)?,
            }
        }
        let mut catalog = crate::catalog::Catalog::empty();
        for model in &self.models {
            catalog.insert(model.clone())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
local_only = false

[[provider]]
name = "openrouter"
kind = "openai-chat"
base_url = "https://openrouter.ai/api/v1"
api_key_env = "OPENROUTER_API_KEY"

[[provider]]
name = "ollama"
kind = "ollama"
base_url = "http://127.0.0.1:11435"

[[model]]
provider = "ollama"
id = "qwen3:8b"
context_window = 32768
max_output = 4096
tools = true

[alias]
fast = "anthropic/claude-haiku-5-5"
default = "route:reliable"

[[route]]
name = "reliable"
candidates = ["anthropic/claude-opus-5-5", "openrouter/openai/some-model", "ollama/qwen3:8b"]
require = ["tools"]
order = "best"

[[route]]
name = "tiered"
candidates = ["anthropic/claude-sonnet-5-5"]
start = ["anthropic/claude-opus-5-5"]
continue = ["anthropic/claude-haiku-5-5"]
"#;

    #[test]
    fn a_full_file_is_read() {
        let config = ProvidersConfig::parse(SAMPLE).unwrap();
        assert_eq!(config.providers.len(), 2);
        assert_eq!(config.models[0].id, "qwen3:8b");
        assert_eq!(config.alias["fast"], "anthropic/claude-haiku-5-5");
        let route = config.route("reliable").unwrap();
        assert_eq!((route.require.clone(), route.order), (vec![Capability::Tools], Order::Best));
        assert_eq!(config.route("tiered").unwrap().continue_.as_deref().unwrap()[0], "anthropic/claude-haiku-5-5");
    }

    #[test]
    fn no_file_means_the_built_in_providers_and_nothing_else() {
        let config = ProvidersConfig::load(Path::new("/definitely/not/here/providers.toml")).unwrap();
        assert_eq!(config, ProvidersConfig::default());
        let names: Vec<String> = config.instances().into_iter().map(|p| p.name).collect();
        assert_eq!(names, vec!["anthropic", "openai", "gemini", "ollama"]);
    }

    #[test]
    fn a_configured_provider_replaces_the_built_in_one_of_the_same_name_and_others_are_added() {
        let config = ProvidersConfig::parse(SAMPLE).unwrap();
        let instances = config.instances();
        let ollama = instances.iter().find(|p| p.name == "ollama").unwrap();
        assert_eq!(ollama.url(), "http://127.0.0.1:11435");
        assert!(ollama.is_local());
        assert!(instances.iter().any(|p| p.name == "openrouter" && !p.is_local()));
        assert_eq!(instances.len(), 5);
    }

    #[test]
    fn a_key_in_the_file_is_refused_with_a_message_that_says_where_keys_go() {
        for line in ["api_key = \"sk-abc\"", "token = \"x\"", "Password = \"x\"", "key = \"x\""] {
            let text = format!("[[provider]]\nname = \"x\"\nkind = \"openai\"\n{line}\n");
            let error = ProvidersConfig::parse(&text).unwrap_err();
            assert!(
                error.contains("must not hold keys") && error.contains("zorix provider set-key"),
                "{line}: {error}"
            );
        }
        // The name of an environment variable is fine, and is not a key.
        assert!(
            ProvidersConfig::parse("[[provider]]\nname = \"x\"\nkind = \"openai\"\napi_key_env = \"MY_KEY\"\n").is_ok()
        );
    }

    #[test]
    fn mistakes_are_explained() {
        let cases = [
            (
                "[[provider]]\nname = \"a\"\nkind = \"openai\"\n[[provider]]\nname = \"a\"\nkind = \"gemini\"\n",
                "listed twice",
            ),
            ("[[provider]]\nname = \"sim\"\nkind = \"openai\"\n", "reserved"),
            ("[[provider]]\nname = \"mine\"\nkind = \"sim\"\n", "reserved"),
            ("[[provider]]\nname = \"Bad Name\"\nkind = \"openai\"\n", "not a valid provider name"),
            ("[[provider]]\nname = \"x\"\nkind = \"openai\"\nbase_url = \"http://example.com\"\n", "plain HTTP"),
            (
                "[[provider]]\nname = \"x\"\nkind = \"openai\"\napi_key_env = \"lower case\"\n",
                "environment variable name",
            ),
            ("[[provider]]\nname = \"x\"\nkind = \"nonsense\"\n", "not valid"),
            ("[[route]]\nname = \"r\"\ncandidates = []\n", "no candidates"),
            ("[[route]]\nname = \"r\"\ncandidates = [\"nobody/model\"]\n", "no provider called"),
            ("[[route]]\nname = \"r\"\ncandidates = [\"alias-not-a-model\"]\n", "not “alias-not-a-model”"),
            ("[alias]\na = \"b\"\n", "another alias"),
            ("[alias]\na = \"route:missing\"\n", "no route called"),
            ("[alias]\nBAD = \"anthropic/x\"\n", "not a valid alias name"),
            ("[[model]]\nprovider = \"x\"\nid = \"m\"\ncontext_window = 10\nmax_output = 20\n", "output limit"),
            ("unknown_field = 1\n", "not valid"),
        ];
        for (text, wanted) in cases {
            let error = ProvidersConfig::parse(text).unwrap_err();
            assert!(error.contains(wanted), "{text:?} → {error}");
        }
    }

    #[test]
    fn selectors_are_parsed_the_way_a_person_writes_them() {
        assert_eq!(
            Selector::parse("anthropic/claude-opus-5-5").unwrap(),
            Selector::Model { provider: "anthropic".into(), model: "claude-opus-5-5".into() }
        );
        assert_eq!(
            Selector::parse("openrouter/openai/some-model").unwrap(),
            Selector::Model { provider: "openrouter".into(), model: "openai/some-model".into() }
        );
        assert_eq!(
            Selector::parse("ollama/qwen3:8b").unwrap(),
            Selector::Model { provider: "ollama".into(), model: "qwen3:8b".into() }
        );
        assert_eq!(Selector::parse("route:reliable").unwrap(), Selector::Route("reliable".into()));
        assert_eq!(Selector::parse("fast").unwrap(), Selector::Alias("fast".into()));
        for bad in ["", "  ", "anthropic/", "Route:X", "UPPER/model", "route:"] {
            assert!(Selector::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn saving_and_loading_round_trips_atomically_and_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(dir.path().join("nested").as_path());
        let config = ProvidersConfig::parse(SAMPLE).unwrap();
        config.save(&path).unwrap();
        assert_eq!(ProvidersConfig::load(&path).unwrap(), config);
        let names: Vec<_> =
            std::fs::read_dir(path.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from("providers.toml")]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn an_invalid_configuration_is_never_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(dir.path());
        let mut config = ProvidersConfig::default();
        config.alias.insert("a".into(), "b".into());
        assert!(config.save(&path).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn what_is_written_never_contains_a_key_field() {
        let text = toml::to_string_pretty(&ProvidersConfig::parse(SAMPLE).unwrap()).unwrap();
        assert!(!text.contains("api_key =") && !text.contains("token"), "{text}");
        assert!(text.contains("api_key_env"));
    }
}
