//! The estimate of a run before it starts (ADR-0045), made the same way for the command line and the desktop app.

use zorix_core::{AgentId, EstimateInput, MAX_SAMPLES, RunEstimate, TenantId, estimate_run};
use zorix_providers::Router;
use zorix_store::SqliteRepository;

use crate::HubError;

/// What a run is about to be: who runs it, on which model, and how big its input is.
#[derive(Clone, Debug, Default)]
pub struct Planned<'a> {
    /// The agent that runs it, when one does.
    pub agent: Option<AgentId>,
    /// The model the run will name (`provider/model`, a route, an alias or `demo`). `None` when the agent's own choice applies: the history of
    /// the agent's runs on any model is used, and the cost is what those runs cost.
    pub model: Option<&'a str>,
    /// Characters of the prompt.
    pub prompt_chars: usize,
    /// Characters of the files attached to it.
    pub attached_chars: usize,
}

/// Estimate the cost, the tokens and the time of a run from what similar runs took.
pub async fn estimate(
    repository: &SqliteRepository,
    router: &Router,
    tenant: &TenantId,
    planned: &Planned<'_>,
) -> Result<RunEstimate, HubError> {
    let samples = repository
        .recent_samples(tenant, planned.agent, planned.model, MAX_SAMPLES)
        .await
        .map_err(|e| HubError(e.to_string()))?;
    let input =
        EstimateInput { prompt_chars: planned.prompt_chars, attached_chars: planned.attached_chars, token_factor: 1.0 };
    let model = planned.model;
    Ok(estimate_run(&samples, &input, |input, output| model.and_then(|m| router.price_of(m, input, output))))
}
