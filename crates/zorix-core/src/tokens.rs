//! Token estimates that learn from what providers report (ADR-0016).

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

#[cfg(test)]
mod tests {
    use super::*;

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
