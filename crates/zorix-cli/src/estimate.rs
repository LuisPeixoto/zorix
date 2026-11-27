//! `zorix run --estimate`: what a run would probably cost and how long it would take, from what similar runs took. Nothing is started and
//! nothing is locked, so it works while another ZORIX program is open.

use std::path::Path;

use zorix_core::{AgentSlug, AgentStore, EstimateBasis, RunEstimate, StepPhase, TenantId};
use zorix_hub::estimate::{Planned, estimate as estimate_run};
use zorix_store::{SqliteAgentStore, database_path};

use crate::app::{CliError, open_reader, open_router};
use crate::cli::RunArgs;
use crate::follow::{Exit, Printer};

pub async fn estimate(home: &Path, tenant: &TenantId, printer: Printer, args: &RunArgs) -> Result<Exit, CliError> {
    let router = open_router(home, &args.model)?;
    let repository = open_reader(home)?;
    // The agent, when one runs it; and the model, which is what was asked for, else the default, else the simulated model.
    let agent = match &args.agent {
        Some(slug) => {
            let slug = AgentSlug::new(slug).map_err(|e| CliError(e.to_string()))?;
            let store = SqliteAgentStore::open(&database_path(home))?;
            let found = store.agent_by_slug(tenant, &slug).await?;
            Some(found.ok_or_else(|| CliError(format!("There is no agent called {slug}.")))?.id)
        }
        None => None,
    };
    let model = match (&args.model.model, &agent) {
        (Some(choice), _) => Some(choice.label()),
        // An agent has its own choice: its runs on any model are what is known about it.
        (None, Some(_)) => None,
        (None, None) if router.resolve("default", StepPhase::Start).is_ok() => Some("default".to_string()),
        (None, None) => Some("demo".to_string()),
    };
    let planned =
        Planned { agent, model: model.as_deref(), prompt_chars: args.prompt.chars().count(), attached_chars: 0 };
    let estimate = estimate_run(&repository, &router, tenant, &planned).await.map_err(|e| CliError(e.to_string()))?;
    if printer.json {
        println!("{}", serde_json::json!({"model": model, "estimate": estimate}));
    } else {
        println!("{}", sentence(&estimate, model.as_deref()));
    }
    Ok(Exit::Done)
}

/// The estimate as one or two sentences.
fn sentence(estimate: &RunEstimate, model: Option<&str>) -> String {
    let cost = match estimate.cost_micros {
        Some(micros) => format!("about ${:.2}", micros as f64 / 1_000_000.0),
        None => "a cost that is not known (the model has no price)".to_string(),
    };
    let seconds = estimate.active_ms / 1_000;
    let time = if seconds >= 90 { format!("about {} min", (seconds + 30) / 60) } else { format!("about {seconds} s") };
    let basis = match estimate.basis {
        EstimateBasis::History { runs } => format!("From the median of your last {runs} similar runs."),
        EstimateBasis::Rough => "A rough guess: fewer than three similar runs to learn from.".to_string(),
    };
    format!(
        "Expected {cost} and {time}{} ({} tokens in, {} out). {basis}",
        model.map(|m| format!(" on {m}")).unwrap_or_default(),
        estimate.input_tokens,
        estimate.output_tokens
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sentence_says_what_it_is_based_on_and_never_invents_a_cost() {
        let known = RunEstimate {
            basis: EstimateBasis::History { runs: 12 },
            input_tokens: 9_000,
            output_tokens: 1_200,
            active_ms: 185_000,
            cost_micros: Some(350_000),
        };
        let text = sentence(&known, Some("anthropic/claude-opus-5-5"));
        assert!(
            text.contains("about $0.35") && text.contains("about 3 min") && text.contains("last 12 similar runs"),
            "{text}"
        );
        let unknown = RunEstimate { basis: EstimateBasis::Rough, cost_micros: None, active_ms: 18_000, ..known };
        let text = sentence(&unknown, None);
        assert!(text.contains("not known") && text.contains("about 18 s") && text.contains("rough guess"), "{text}");
        assert!(!text.contains('$'), "{text}");
    }
}
