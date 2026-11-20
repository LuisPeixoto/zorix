//! The deterministic checks: what an execution did, judged by rules and nothing else.

use serde_json::Value;
use zorix_core::{CheckOutcome, EvalCase, EvalCheck, ExecutionDetail, Permission, ToolCallStatus};

/// Does a tool id match a pattern (an id, or a prefix ending in `*`)?
fn tool_matches(pattern: &str, tool: &str) -> bool {
    Permission::use_tool(pattern).covers(&Permission::use_tool(tool))
}

/// The answer as text.
fn answer(detail: &ExecutionDetail) -> Option<&str> {
    detail.execution.result.as_ref().map(|r| r.text.as_str())
}

/// The answer as JSON: its structured form, or its text read as JSON (also from inside a code fence or after a sentence).
fn answer_json(detail: &ExecutionDetail) -> Option<Value> {
    let result = detail.execution.result.as_ref()?;
    if let Some(structured) = &result.structured {
        return Some(structured.clone());
    }
    let text = result.text.trim();
    if let Ok(value) = serde_json::from_str(text) {
        return Some(value);
    }
    let (start, end) = (text.find(['{', '[']), text.rfind(['}', ']']));
    match (start, end) {
        (Some(s), Some(e)) if s < e => serde_json::from_str(&text[s..=e]).ok(),
        _ => None,
    }
}

fn outcome(check: impl Into<String>, passed: bool, detail: impl Into<String>) -> CheckOutcome {
    CheckOutcome { check: check.into(), passed, detail: detail.into() }
}

fn one(check: &EvalCheck, detail: &ExecutionDetail) -> CheckOutcome {
    let calls = &detail.tool_calls;
    let usage = &detail.execution.usage;
    match check {
        EvalCheck::Contains { text, ignore_case } | EvalCheck::NotContains { text, ignore_case } => {
            let wants = matches!(check, EvalCheck::Contains { .. });
            let label = format!("{} “{text}”", if wants { "contains" } else { "does not contain" });
            let Some(found) = answer(detail) else { return outcome(label, false, "There is no answer.") };
            let has = if *ignore_case {
                found.to_lowercase().contains(&text.to_lowercase())
            } else {
                found.contains(text.as_str())
            };
            outcome(label, has == wants, if has { "It is in the answer." } else { "It is not in the answer." })
        }
        EvalCheck::Regex { pattern } => {
            let label = format!("matches /{pattern}/");
            let Some(found) = answer(detail) else { return outcome(label, false, "There is no answer.") };
            match regex::RegexBuilder::new(pattern).size_limit(1 << 20).dfa_size_limit(1 << 20).build() {
                Ok(re) => {
                    let hit = re.is_match(found);
                    outcome(label, hit, if hit { "It matches." } else { "It does not match." })
                }
                Err(e) => outcome(label, false, format!("The pattern is not valid: {e}")),
            }
        }
        EvalCheck::JsonSchema { schema } => {
            let label = "fits the JSON Schema";
            let Some(value) = answer_json(detail) else { return outcome(label, false, "The answer is not JSON.") };
            match jsonschema::validator_for(schema) {
                Ok(validator) => match validator.validate(&value) {
                    Ok(()) => outcome(label, true, "It fits."),
                    Err(e) => outcome(label, false, e.to_string()),
                },
                Err(e) => outcome(label, false, format!("The schema is not valid: {e}")),
            }
        }
        EvalCheck::ToolUsed { pattern, min } => {
            let n =
                calls.iter().filter(|c| c.status != ToolCallStatus::Denied && tool_matches(pattern, &c.tool)).count()
                    as u32;
            outcome(format!("used {pattern} at least {min} time(s)"), n >= *min, format!("It was used {n} time(s)."))
        }
        EvalCheck::ToolNotUsed { pattern } => {
            let n = calls.iter().filter(|c| tool_matches(pattern, &c.tool)).count();
            outcome(
                format!("did not use {pattern}"),
                n == 0,
                if n == 0 { "It was not asked for.".to_string() } else { format!("It was asked for {n} time(s).") },
            )
        }
        EvalCheck::MaxCostUsd { usd } => {
            let spent = usage.cost_micros as f64 / 1_000_000.0;
            outcome(format!("cost at most ${usd:.4}"), spent <= *usd, format!("It cost ${spent:.4}."))
        }
        EvalCheck::MaxSteps { steps } => {
            let n = usage.model_calls;
            outcome(format!("took at most {steps} step(s)"), n <= *steps, format!("It took {n}."))
        }
        EvalCheck::MaxToolCalls { calls: most } => {
            let n = calls.len() as u32;
            outcome(format!("made at most {most} tool call(s)"), n <= *most, format!("It made {n}."))
        }
        EvalCheck::Delegated { agent, min } => {
            // The record of each delegate call lists the agent every task went to, whatever came of it.
            let asked: u32 = calls
                .iter()
                .filter(|c| c.tool == zorix_core::delegation::TOOL)
                .filter_map(|c| c.result.as_ref().and_then(|r| r["results"].as_array()))
                .flatten()
                .filter(|r| r["agent"].as_str() == Some(agent.as_str()))
                .count() as u32;
            outcome(
                format!("asked {agent} at least {min} time(s)"),
                asked >= *min,
                format!("It was asked {asked} time(s)."),
            )
        }
        EvalCheck::Status { state } => {
            let now = detail.execution.state.as_str();
            outcome(format!("ended {state}"), now == state, format!("It ended {now}."))
        }
    }
}

/// Run every check of a case against what happened. A case with no `status` check is expected to complete.
pub fn run_checks(case: &EvalCase, detail: &ExecutionDetail) -> Vec<CheckOutcome> {
    let mut results: Vec<CheckOutcome> = case.checks.iter().map(|c| one(c, detail)).collect();
    if !case.checks.iter().any(|c| matches!(c, EvalCheck::Status { .. })) {
        results.insert(0, one(&EvalCheck::Status { state: "completed".into() }, detail));
    }
    results
}
