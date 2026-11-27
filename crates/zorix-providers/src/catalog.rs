//! The model catalogue: what each model can do and what it costs (ADR-0010).
//!
//! A small curated list ships with ZORIX, dated, and covers only what has a published price. Everything else comes from
//! discovery at run time (Ollama, Gemini, any server's `/v1/models`) or from the person's own configuration. Where a price
//! is unknown the cost is unknown: it is never guessed.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use zorix_core::{ModelProfile, Usage};

const EMBEDDED: &str = include_str!("../data/models.json");

/// How fast a model usually answers, for ordering routes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Latency {
    /// Answers quickly.
    Fast,
    /// A typical speed.
    #[default]
    Standard,
    /// Thorough and slow.
    Slow,
}

/// What a model costs, in US dollars per million tokens.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Price {
    /// Input tokens that were not served from the cache.
    pub input: f64,
    /// Output tokens (reasoning tokens are billed as output).
    pub output: f64,
    /// Input tokens served from the cache. Defaults to the input price.
    #[serde(default)]
    pub cache_read: Option<f64>,
    /// Input tokens written to the cache. Defaults to the input price.
    #[serde(default)]
    pub cache_write: Option<f64>,
}

impl Price {
    /// The cost of a call, in millionths of a dollar.
    ///
    /// `Usage::input_tokens` counts only tokens that were neither read from nor written to the cache; adapters normalise
    /// each provider's numbers into that shape.
    pub fn cost_micros(&self, usage: &Usage) -> u64 {
        let per_token = |price_per_million: f64, tokens: u64| price_per_million * tokens as f64; // micros = $/M * tokens
        let micros = per_token(self.input, usage.input_tokens)
            + per_token(self.cache_read.unwrap_or(self.input), usage.cache_read_tokens)
            + per_token(self.cache_write.unwrap_or(self.input), usage.cache_write_tokens)
            + per_token(self.output, usage.output_tokens);
        micros.round() as u64
    }
}

/// Everything known about one model of one provider.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelInfo {
    /// The provider instance, such as `anthropic` or `ollama`.
    pub provider: String,
    /// The model id the provider uses.
    pub id: String,
    /// A name for people.
    #[serde(default, rename = "name")]
    pub display_name: String,
    /// The context window, in tokens.
    pub context_window: u64,
    /// The most tokens it writes in one answer.
    pub max_output: u64,
    /// It can ask for tools.
    #[serde(default)]
    pub tools: bool,
    /// It can read images.
    #[serde(default)]
    pub vision: bool,
    /// It reasons before answering.
    #[serde(default)]
    pub thinking: bool,
    /// How fast it usually is.
    #[serde(default)]
    pub latency: Latency,
    /// A rough quality tier from 1 (small) to 5 (the most capable), to order routes by quality.
    #[serde(default)]
    pub tier: u8,
    /// What it costs, when that is known.
    #[serde(default)]
    pub price: Option<Price>,
    /// True when the numbers were assumed rather than reported (for example a server that does not say its window).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub assumed: bool,
}

impl ModelInfo {
    /// What the runtime needs to size the context.
    pub fn profile(&self) -> ModelProfile {
        ModelProfile {
            context_window: self.context_window,
            max_output: self.max_output,
            tools: self.tools,
            vision: self.vision,
            // A model that runs here is free whatever the catalogue says; the router knows where a model runs and says so.
            priced: self.price.is_some(),
        }
    }

    /// `provider/model`.
    pub fn reference(&self) -> String {
        format!("{}/{}", self.provider, self.id)
    }

    fn check(&self) -> Result<(), String> {
        let name = self.reference();
        if self.provider.trim().is_empty() || self.id.trim().is_empty() {
            return Err("A model needs a provider and an id.".into());
        }
        if self.context_window == 0 || self.max_output == 0 {
            return Err(format!("{name}: the context window and the output limit must be above zero."));
        }
        if self.max_output > self.context_window {
            return Err(format!("{name}: the output limit is larger than the context window."));
        }
        if let Some(price) = &self.price
            && [price.input, price.output, price.cache_read.unwrap_or(0.0), price.cache_write.unwrap_or(0.0)]
                .iter()
                .any(|p| !p.is_finite() || *p < 0.0)
        {
            return Err(format!("{name}: a price is negative or not a number."));
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct File {
    as_of: String,
    models: Vec<ModelInfo>,
}

/// The catalogue.
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    as_of: Option<String>,
    models: BTreeMap<(String, String), ModelInfo>,
}

impl Catalog {
    /// The curated catalogue that ships with ZORIX.
    pub fn embedded() -> Self {
        Self::from_json(EMBEDDED).expect("the embedded catalogue is valid (a test checks it)")
    }

    /// An empty catalogue.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Read a catalogue from JSON of the same shape as the embedded one.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let file: File = serde_json::from_str(text).map_err(|e| format!("The model catalogue is not valid: {e}"))?;
        let mut catalog = Catalog { as_of: Some(file.as_of), models: BTreeMap::new() };
        for model in file.models {
            catalog.insert(model)?;
        }
        Ok(catalog)
    }

    /// Add a model, replacing one with the same provider and id. Refuses numbers that make no sense.
    pub fn insert(&mut self, model: ModelInfo) -> Result<(), String> {
        model.check()?;
        self.models.insert((model.provider.clone(), model.id.clone()), model);
        Ok(())
    }

    /// When the curated prices were taken.
    pub fn as_of(&self) -> Option<&str> {
        self.as_of.as_deref()
    }

    /// One model.
    pub fn get(&self, provider: &str, model: &str) -> Option<&ModelInfo> {
        self.models.get(&(provider.to_string(), model.to_string()))
    }

    /// Every model, ordered by provider and id.
    pub fn list(&self) -> impl Iterator<Item = &ModelInfo> {
        self.models.values()
    }

    /// The models of one provider.
    pub fn of_provider<'a>(&'a self, provider: &'a str) -> impl Iterator<Item = &'a ModelInfo> {
        self.models.values().filter(move |m| m.provider == provider)
    }

    /// The cost of a call in millionths of a dollar, or `None` when the model or its price is unknown.
    pub fn cost_micros(&self, provider: &str, model: &str, usage: &Usage) -> Option<u64> {
        self.get(provider, model)?.price.map(|price| price.cost_micros(usage))
    }

    /// Add what another catalogue knows (discovery, configuration). Existing entries are replaced.
    pub fn merge(&mut self, other: Catalog) {
        for (key, model) in other.models {
            self.models.insert(key, model);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: u64, output: u64) -> Usage {
        Usage { input_tokens: input, output_tokens: output, ..Usage::default() }
    }

    #[test]
    fn the_embedded_catalogue_is_valid_dated_and_has_the_default_model() {
        let catalog = Catalog::embedded();
        assert_eq!(catalog.as_of(), Some("2026-11-03"));
        let opus = catalog.get("anthropic", "claude-opus-5-5").expect("the default model");
        assert_eq!((opus.context_window, opus.max_output, opus.tools, opus.vision), (1_000_000, 128_000, true, true));
        assert!(catalog.list().count() >= 6);
        assert!(catalog.list().all(|m| m.check().is_ok() && m.price.is_some() && (1..=5).contains(&m.tier)));
    }

    #[test]
    fn cost_is_exact_in_millionths_of_a_dollar() {
        let catalog = Catalog::embedded();
        // Opus 5.5: $4 in, $20 out per million. 1000 in + 500 out = $0.004 + $0.010 = $0.014.
        assert_eq!(catalog.cost_micros("anthropic", "claude-opus-5-5", &usage(1_000, 500)), Some(14_000));
        // Haiku 5.5: $0.10 in, $0.50 out per million. One million of each = $0.60.
        assert_eq!(catalog.cost_micros("anthropic", "claude-haiku-5-5", &usage(1_000_000, 1_000_000)), Some(600_000));
        assert_eq!(catalog.cost_micros("anthropic", "claude-opus-5-5", &usage(0, 0)), Some(0));
    }

    #[test]
    fn cached_tokens_use_their_own_price_or_fall_back_to_the_input_price() {
        let catalog = Catalog::embedded();
        let cached = Usage { input_tokens: 0, cache_read_tokens: 1_000_000, ..Usage::default() };
        assert_eq!(
            catalog.cost_micros("anthropic", "claude-opus-5-5", &cached),
            Some(200_000),
            "cache read is $0.20 per million"
        );
        assert_eq!(
            catalog.cost_micros("anthropic", "claude-opus-5", &cached),
            Some(5_000_000),
            "no cache price: the input price"
        );
        let written = Usage { cache_write_tokens: 1_000_000, ..Usage::default() };
        assert_eq!(catalog.cost_micros("anthropic", "claude-opus-5-5", &written), Some(4_000_000));
    }

    #[test]
    fn an_unknown_model_or_price_is_unknown_never_zero() {
        let catalog = Catalog::embedded();
        assert_eq!(catalog.cost_micros("anthropic", "claude-future-9", &usage(1, 1)), None);
        assert_eq!(catalog.cost_micros("nobody", "x", &usage(1, 1)), None);
        let mut custom = Catalog::empty();
        custom
            .insert(ModelInfo {
                provider: "ollama".into(),
                id: "qwen3".into(),
                display_name: String::new(),
                context_window: 32_768,
                max_output: 4_096,
                tools: true,
                vision: false,
                thinking: false,
                latency: Latency::Standard,
                tier: 2,
                price: None,
                assumed: false,
            })
            .unwrap();
        assert_eq!(custom.cost_micros("ollama", "qwen3", &usage(100, 100)), None);
    }

    #[test]
    fn nonsense_entries_are_refused() {
        let mut bad = Catalog::embedded().get("anthropic", "claude-opus-5-5").unwrap().clone();
        let mut catalog = Catalog::empty();
        bad.max_output = bad.context_window + 1;
        assert!(catalog.insert(bad.clone()).unwrap_err().contains("output limit"));
        bad.max_output = 0;
        assert!(catalog.insert(bad.clone()).is_err());
        bad.max_output = 10;
        bad.price = Some(Price { input: -1.0, output: 1.0, cache_read: None, cache_write: None });
        assert!(catalog.insert(bad.clone()).unwrap_err().contains("price"));
        bad.provider = " ".into();
        assert!(catalog.insert(bad).is_err());
        assert!(Catalog::from_json("{").is_err());
    }

    #[test]
    fn merging_lets_the_person_override_the_curated_entry() {
        let mut catalog = Catalog::embedded();
        let mut other = Catalog::empty();
        let mut mine = catalog.get("anthropic", "claude-opus-5-5").unwrap().clone();
        mine.price = Some(Price { input: 1.0, output: 2.0, cache_read: None, cache_write: None });
        other.insert(mine).unwrap();
        catalog.merge(other);
        assert_eq!(catalog.cost_micros("anthropic", "claude-opus-5-5", &usage(1_000_000, 0)), Some(1_000_000));
    }

    #[test]
    fn a_profile_and_a_reference_come_from_the_entry() {
        let catalog = Catalog::embedded();
        let model = catalog.get("anthropic", "claude-haiku-4-5").unwrap();
        assert_eq!(model.profile().context_window, 200_000);
        assert_eq!(model.reference(), "anthropic/claude-haiku-4-5");
        assert_eq!(catalog.of_provider("anthropic").count(), catalog.list().count());
    }
}
