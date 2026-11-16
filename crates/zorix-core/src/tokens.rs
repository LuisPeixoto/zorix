//! Token estimates that learn from what providers report (ADR-0016).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::model::StepPhase;

/// One comparison of the estimate with the real prompt size.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TokenSample {
    /// The model reference that was asked for (`default`, `route:fast`, `anthropic/claude-opus-5-5`…).
    pub requested: String,
    /// The `provider/model` that answered, when known.
    pub answered: Option<String>,
    /// What the estimate said: messages plus tool definitions.
    pub estimated: u64,
    /// What the provider counted: input plus cache reads and writes.
    pub actual: u64,
    /// Whether this was the first call of the execution, a continued step or a retry.
    pub phase: StepPhase,
}

/// Where the correction for a model is kept.
pub trait TokenCalibration: Send + Sync {
    /// How many real tokens one estimated token is worth for `model`. 1.0 until there is enough data.
    fn factor(&self, model: &str) -> f64;
    /// Learn from one call.
    fn observe(&self, sample: TokenSample);
}

/// The default: no correction.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoCalibration;

impl TokenCalibration for NoCalibration {
    fn factor(&self, _model: &str) -> f64 {
        1.0
    }

    fn observe(&self, _sample: TokenSample) {}
}

/// The smallest estimate that teaches anything: short prompts are dominated by the provider's framing.
pub const MIN_ESTIMATE: u64 = 512;
/// Samples needed before a factor is used.
pub const MIN_SAMPLES: u32 = 3;
/// The ratio is clamped to this range, so one odd call cannot wreck the budget.
pub const MIN_FACTOR: f64 = 0.5;
/// See [`MIN_FACTOR`].
pub const MAX_FACTOR: f64 = 3.0;
/// How fast the factor follows new samples once it has settled.
pub const ALPHA: f64 = 0.2;

/// What is known about one model.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Calibration {
    /// Real tokens per estimated token.
    pub factor: f64,
    /// How many samples went into it.
    pub samples: u32,
}

impl Default for Calibration {
    fn default() -> Self {
        Self { factor: 1.0, samples: 0 }
    }
}

/// The ratio a sample says, or `None` when the sample teaches nothing (ADR-0016): simulated models, short prompts, no
/// usage, and the continued steps of Ollama, whose count leaves out tokens it kept from the previous call.
pub fn sample_ratio(sample: &TokenSample) -> Option<f64> {
    let named = |name: &str| name == "demo" || name == "script" || name.starts_with("sim/");
    if named(&sample.requested) || sample.answered.as_deref().is_some_and(named) {
        return None;
    }
    let ollama =
        sample.requested.starts_with("ollama/") || sample.answered.as_deref().is_some_and(|a| a.starts_with("ollama/"));
    if ollama && sample.phase != StepPhase::Start {
        return None;
    }
    if sample.estimated < MIN_ESTIMATE || sample.actual == 0 {
        return None;
    }
    Some((sample.actual as f64 / sample.estimated as f64).clamp(MIN_FACTOR, MAX_FACTOR))
}

/// Fold one ratio into what is known. Early samples average; later ones follow an exponential moving average.
/// `weight` (0 to 1) lets weaker evidence, such as old events, count for less.
pub fn fold(known: Calibration, ratio: f64, weight: f64) -> Calibration {
    let rate = ALPHA.max(1.0 / (known.samples as f64 + 1.0)) * weight.clamp(0.0, 1.0);
    let factor = (known.factor + rate * (ratio - known.factor)).clamp(MIN_FACTOR, MAX_FACTOR);
    Calibration { factor, samples: known.samples + 1 }
}

/// Calibration kept in memory. The store crate wraps it to persist what it learns.
#[derive(Default)]
pub struct InMemoryCalibration {
    known: std::sync::RwLock<HashMap<String, Calibration>>,
}

impl InMemoryCalibration {
    /// Nothing known yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Start from what was saved.
    pub fn with(entries: impl IntoIterator<Item = (String, Calibration)>) -> Self {
        Self { known: std::sync::RwLock::new(entries.into_iter().collect()) }
    }

    /// Learn from one call, counting it as `weight` of a sample. Returns the entries that changed, to be saved.
    pub fn observe_weighted(&self, sample: &TokenSample, weight: f64) -> Vec<(String, Calibration)> {
        let Some(ratio) = sample_ratio(sample) else { return Vec::new() };
        let mut keys = vec![sample.requested.clone()];
        if let Some(answered) = &sample.answered
            && answered != &sample.requested
        {
            keys.push(answered.clone());
        }
        let mut known = self.known.write().unwrap_or_else(|e| e.into_inner());
        keys.into_iter()
            .map(|key| {
                let next = fold(known.get(&key).copied().unwrap_or_default(), ratio, weight);
                known.insert(key.clone(), next);
                (key, next)
            })
            .collect()
    }

    /// What is known about a model.
    pub fn get(&self, model: &str) -> Option<Calibration> {
        self.known.read().unwrap_or_else(|e| e.into_inner()).get(model).copied()
    }

    /// Every model with its calibration, sorted by name.
    pub fn all(&self) -> Vec<(String, Calibration)> {
        let mut all: Vec<_> =
            self.known.read().unwrap_or_else(|e| e.into_inner()).iter().map(|(k, v)| (k.clone(), *v)).collect();
        all.sort_by(|a, b| a.0.cmp(&b.0));
        all
    }

    /// True when nothing has been learned.
    pub fn is_empty(&self) -> bool {
        self.known.read().unwrap_or_else(|e| e.into_inner()).is_empty()
    }
}

impl TokenCalibration for InMemoryCalibration {
    fn factor(&self, model: &str) -> f64 {
        match self.get(model) {
            Some(known) if known.samples >= MIN_SAMPLES => known.factor,
            _ => 1.0,
        }
    }

    fn observe(&self, sample: TokenSample) {
        self.observe_weighted(&sample, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(requested: &str, answered: Option<&str>, estimated: u64, actual: u64, phase: StepPhase) -> TokenSample {
        TokenSample { requested: requested.into(), answered: answered.map(str::to_string), estimated, actual, phase }
    }

    #[test]
    fn samples_that_teach_nothing_are_skipped() {
        let ok = sample("anthropic/claude", None, 1_000, 2_000, StepPhase::Start);
        assert_eq!(sample_ratio(&ok), Some(2.0));
        for skipped in [
            sample("demo", None, 1_000, 2_000, StepPhase::Start),
            sample("script", None, 1_000, 2_000, StepPhase::Start),
            sample("default", Some("sim/demo"), 1_000, 2_000, StepPhase::Start),
            sample("anthropic/claude", None, 511, 2_000, StepPhase::Start),
            sample("anthropic/claude", None, 1_000, 0, StepPhase::Start),
            sample("ollama/qwen3", None, 1_000, 2_000, StepPhase::Continue),
            sample("default", Some("ollama/qwen3"), 1_000, 2_000, StepPhase::Retry),
        ] {
            assert_eq!(sample_ratio(&skipped), None, "{skipped:?}");
        }
        assert_eq!(
            sample_ratio(&sample("ollama/qwen3", None, 1_000, 2_000, StepPhase::Start)),
            Some(2.0),
            "the first call counts"
        );
        assert_eq!(
            sample_ratio(&sample("a/b", None, 1_000, 9_000, StepPhase::Start)),
            Some(MAX_FACTOR),
            "clamped high"
        );
        assert_eq!(sample_ratio(&sample("a/b", None, 1_000, 10, StepPhase::Start)), Some(MIN_FACTOR), "clamped low");
    }

    #[test]
    fn the_factor_is_used_only_after_enough_samples_and_then_follows_the_data() {
        let calibration = InMemoryCalibration::new();
        let s = sample("anthropic/claude", None, 1_000, 1_500, StepPhase::Start);
        assert_eq!(calibration.factor("anthropic/claude"), 1.0);
        calibration.observe(s.clone());
        calibration.observe(s.clone());
        assert_eq!(calibration.factor("anthropic/claude"), 1.0, "two samples are not enough");
        calibration.observe(s.clone());
        assert!((calibration.factor("anthropic/claude") - 1.5).abs() < 1e-9, "the early samples average exactly");
        // The model changes (or the text does): the factor follows slowly.
        for _ in 0..40 {
            calibration.observe(sample("anthropic/claude", None, 1_000, 2_500, StepPhase::Start));
        }
        let f = calibration.factor("anthropic/claude");
        assert!(f > 2.4 && f <= 2.5, "{f}");
        assert_eq!(calibration.factor("unknown/model"), 1.0);
    }

    #[test]
    fn a_sample_is_kept_under_the_requested_name_and_the_serving_model() {
        let calibration = InMemoryCalibration::new();
        for _ in 0..3 {
            calibration.observe(sample("route:fast", Some("anthropic/claude-haiku"), 1_000, 1_200, StepPhase::Start));
        }
        assert!((calibration.factor("route:fast") - 1.2).abs() < 1e-9);
        assert!((calibration.factor("anthropic/claude-haiku") - 1.2).abs() < 1e-9);
        let names: Vec<String> = calibration.all().into_iter().map(|(k, _)| k).collect();
        assert_eq!(names, vec!["anthropic/claude-haiku".to_string(), "route:fast".to_string()]);
    }

    #[test]
    fn weaker_evidence_moves_the_factor_less() {
        let strong = InMemoryCalibration::new();
        let weak = InMemoryCalibration::new();
        let s = sample("a/b", None, 1_000, 2_000, StepPhase::Start);
        for _ in 0..3 {
            strong.observe_weighted(&s, 1.0);
            weak.observe_weighted(&s, 0.5);
        }
        assert!(strong.factor("a/b") > weak.factor("a/b") && weak.factor("a/b") > 1.0);
        assert!(weak.observe_weighted(&sample("demo", None, 9_000, 9_000, StepPhase::Start), 1.0).is_empty());
    }

    #[test]
    fn no_calibration_leaves_estimates_alone() {
        let none = NoCalibration;
        none.observe(TokenSample {
            requested: "x".into(),
            answered: None,
            estimated: 1_000,
            actual: 2_000,
            phase: StepPhase::Start,
        });
        assert_eq!(none.factor("anything"), 1.0);
    }
}
