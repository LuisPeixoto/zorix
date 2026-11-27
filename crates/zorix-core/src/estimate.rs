//! A guess of what a run will cost and how long it will take, made before it starts (ADR-0045).
//!
//! The honest source is what similar runs took: the median of the last few finished runs of the same agent on the same model. With fewer than
//! three of them there is nothing to go on, and the estimate says it is rough and uses fixed guesses for a run of a few steps. The cost is the price
//! of the model for the estimated tokens, and is unknown (never zero) when the model has no price. It is a guess, not a limit: a limit is a
//! setting of the run.

use serde::{Deserialize, Serialize};

/// The fewest finished runs the estimate is built from. Below this it is a rough guess.
pub const MIN_SAMPLES: usize = 3;
/// The most runs it looks at.
pub const MAX_SAMPLES: usize = 20;

/// Tokens of the fixed text every call carries (the platform's instructions and the tool definitions of a typical run).
const ROUGH_FIXED_TOKENS: u64 = 350;
/// How many model calls a run is guessed to make when nothing is known.
const ROUGH_CALLS: u64 = 3;
/// Tokens a model is guessed to write in one call.
const ROUGH_OUTPUT_TOKENS: u64 = 350;
/// Milliseconds one call and its tools are guessed to take.
const ROUGH_CALL_MS: u64 = 6_000;

/// What a finished run took.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSample {
    /// Tokens sent to the model, over the whole run.
    pub input_tokens: u64,
    /// Tokens the model wrote.
    pub output_tokens: u64,
    /// How many model calls there were.
    pub model_calls: u32,
    /// Milliseconds of work (waiting for a person is not counted).
    pub active_ms: u64,
    /// What it cost, in millionths of a dollar (zero when the model had no price).
    pub cost_micros: u64,
}

/// Where an estimate comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EstimateBasis {
    /// The median of this many finished runs of the same agent on the same model.
    History {
        /// How many.
        runs: u32,
    },
    /// Fixed guesses: there were not enough runs to learn from.
    Rough,
}

/// What a run is about to be.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EstimateInput {
    /// Characters of the prompt.
    pub prompt_chars: usize,
    /// Characters of the files attached to the run. They are sent with every call.
    pub attached_chars: usize,
    /// How many real tokens one estimated token is worth for this model (1.0 until calibrated).
    pub token_factor: f64,
}

/// The guess.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunEstimate {
    /// Where it comes from.
    pub basis: EstimateBasis,
    /// Tokens sent to the model, over the whole run.
    pub input_tokens: u64,
    /// Tokens the model will write.
    pub output_tokens: u64,
    /// Milliseconds of work.
    pub active_ms: u64,
    /// Cost in millionths of a dollar. `None` when the model has no known price: unknown, not free.
    pub cost_micros: Option<u64>,
}

fn median(mut values: Vec<u64>) -> u64 {
    values.sort_unstable();
    match values.len() {
        0 => 0,
        n if n % 2 == 1 => values[n / 2],
        n => (values[n / 2 - 1] + values[n / 2]) / 2,
    }
}

fn tokens_of(chars: usize, factor: f64) -> u64 {
    let factor = if factor.is_finite() && factor > 0.0 { factor } else { 1.0 };
    (((chars as f64) / 4.0) * factor).ceil() as u64
}

/// Estimate a run from `samples` (the newest first; at most [`MAX_SAMPLES`] are used). `price` turns tokens into a cost, or `None` for a model with
/// no price.
pub fn estimate_run(
    samples: &[RunSample],
    input: &EstimateInput,
    price: impl Fn(u64, u64) -> Option<u64>,
) -> RunEstimate {
    let samples = &samples[..samples.len().min(MAX_SAMPLES)];
    let attached = tokens_of(input.attached_chars, input.token_factor);
    let (basis, input_tokens, output_tokens, active_ms) = if samples.len() >= MIN_SAMPLES {
        let calls = median(samples.iter().map(|s| u64::from(s.model_calls.max(1))).collect());
        (
            EstimateBasis::History { runs: samples.len() as u32 },
            // What similar runs sent, and what the files add to each call.
            median(samples.iter().map(|s| s.input_tokens).collect()) + attached * calls,
            median(samples.iter().map(|s| s.output_tokens).collect()),
            median(samples.iter().map(|s| s.active_ms).collect()),
        )
    } else {
        let per_call = ROUGH_FIXED_TOKENS + tokens_of(input.prompt_chars, input.token_factor) + attached;
        (EstimateBasis::Rough, per_call * ROUGH_CALLS, ROUGH_OUTPUT_TOKENS * ROUGH_CALLS, ROUGH_CALL_MS * ROUGH_CALLS)
    };
    // The model's price, when it has one. When it does not (a run of an agent, whose model is not named here), what the same runs cost is the
    // best there is, as long as any of them cost anything: a run that cost nothing at all says nothing about its price.
    let cost_micros = price(input_tokens, output_tokens).or_else(|| {
        let costs: Vec<u64> = samples.iter().map(|s| s.cost_micros).collect();
        (samples.len() >= MIN_SAMPLES && costs.iter().any(|c| *c > 0)).then(|| median(costs))
    });
    RunEstimate { basis, input_tokens, output_tokens, active_ms, cost_micros }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(input: u64, output: u64, calls: u32, ms: u64) -> RunSample {
        RunSample { input_tokens: input, output_tokens: output, model_calls: calls, active_ms: ms, cost_micros: 0 }
    }

    fn input(prompt: usize, attached: usize) -> EstimateInput {
        EstimateInput { prompt_chars: prompt, attached_chars: attached, token_factor: 1.0 }
    }

    /// $1 per million input tokens and $2 per million output tokens, as micros: a token costs its price in micros.
    fn priced(i: u64, o: u64) -> Option<u64> {
        Some(i + 2 * o)
    }

    #[test]
    fn with_enough_runs_the_estimate_is_their_median_and_says_how_many() {
        // One run was ten times the others: the median is not moved by it.
        let samples = [
            sample(1_000, 200, 2, 4_000),
            sample(1_200, 220, 2, 5_000),
            sample(12_000, 2_000, 9, 60_000),
            sample(1_100, 210, 2, 4_500),
        ];
        let e = estimate_run(&samples, &input(100, 0), priced);
        assert_eq!(e.basis, EstimateBasis::History { runs: 4 });
        assert_eq!((e.input_tokens, e.output_tokens, e.active_ms), (1_150, 215, 4_750));
        assert_eq!(e.cost_micros, Some(1_150 + 2 * 215));
    }

    #[test]
    fn files_add_their_tokens_to_every_call_of_a_run() {
        let samples = [sample(1_000, 200, 4, 4_000); 3];
        let without = estimate_run(&samples, &input(100, 0), priced);
        let with = estimate_run(&samples, &input(100, 4_000), priced);
        // 4,000 characters are 1,000 tokens, sent with each of the four calls.
        assert_eq!(with.input_tokens - without.input_tokens, 4_000);
        assert_eq!(with.output_tokens, without.output_tokens);
    }

    #[test]
    fn too_few_runs_make_a_rough_guess_that_grows_with_the_prompt_and_says_so() {
        for count in 0..MIN_SAMPLES {
            let samples = vec![sample(1_000, 200, 2, 4_000); count];
            let e = estimate_run(&samples, &input(400, 0), priced);
            assert_eq!(e.basis, EstimateBasis::Rough, "{count} runs");
        }
        let small = estimate_run(&[], &input(400, 0), priced);
        let big = estimate_run(&[], &input(40_000, 0), priced);
        assert!(big.input_tokens > small.input_tokens * 5 && big.cost_micros > small.cost_micros);
        assert_eq!(small.active_ms, 18_000);
    }

    #[test]
    fn a_model_with_no_price_has_an_unknown_cost_never_a_free_one() {
        let e = estimate_run(&[sample(1_000, 200, 2, 4_000); 5], &input(100, 0), |_, _| None);
        assert_eq!(e.cost_micros, None);
        assert!(e.input_tokens > 0, "the tokens and the time are still guessed");
    }

    #[test]
    fn without_a_price_what_the_same_runs_cost_stands_in_when_any_of_them_cost_something() {
        let costly = |cost: u64| RunSample { cost_micros: cost, ..sample(1_000, 200, 2, 4_000) };
        let e = estimate_run(&[costly(300), costly(500), costly(400)], &input(100, 0), |_, _| None);
        assert_eq!(e.cost_micros, Some(400), "the median of what they cost");
        // Fewer than three, or all free: nothing to stand on.
        assert_eq!(estimate_run(&[costly(300), costly(500)], &input(100, 0), |_, _| None).cost_micros, None);
        assert_eq!(estimate_run(&[costly(0), costly(0), costly(0)], &input(100, 0), |_, _| None).cost_micros, None);
    }

    #[test]
    fn only_the_newest_runs_count_and_a_bad_factor_is_ignored() {
        let mut samples = vec![sample(1_000, 100, 2, 1_000); MAX_SAMPLES];
        samples.extend(vec![sample(900_000, 9_000, 50, 9_000_000); 50]);
        let e = estimate_run(&samples, &input(0, 0), priced);
        assert_eq!(e.basis, EstimateBasis::History { runs: MAX_SAMPLES as u32 });
        assert_eq!(e.input_tokens, 1_000);
        let odd = EstimateInput { prompt_chars: 4_000, attached_chars: 0, token_factor: f64::NAN };
        assert_eq!(
            estimate_run(&[], &odd, priced).input_tokens,
            estimate_run(&[], &input(4_000, 0), priced).input_tokens
        );
    }
}
