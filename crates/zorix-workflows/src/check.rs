//! The checks that need the expression language, templates and schedules: what `WorkflowSpec::validate` in the core cannot see.
//!
//! A workflow is checked before it is saved or published, so a typo shows up in the editor and not at three in the morning.

use zorix_core::{StepKind, WorkflowSpec};

use crate::expr::{Expr, Segment, parse_path};
use crate::graph::ancestors;
use crate::schedule::next_fire;
use crate::template;

const ROOTS: [&str; 5] = ["trigger", "steps", "run", "item", "index"];

fn path_ok(spec: &WorkflowSpec, step: &str, path: &[Segment], in_loop: bool, text: &str) -> Result<(), String> {
    let Some(Segment::Key(root)) = path.first() else {
        return Err(format!("Step {step}: “{text}” does not start with a name."));
    };
    if !ROOTS.contains(&root.as_str()) {
        return Err(format!(
            "Step {step}: “{text}” starts with “{root}”. Start with trigger, steps or run{}.",
            if in_loop { " (or item inside a loop)" } else { "" }
        ));
    }
    if (root == "item" || root == "index") && !in_loop {
        return Err(format!("Step {step}: “{text}” is only available in a loop step's prompt."));
    }
    if root == "steps" {
        let Some(Segment::Key(other)) = path.get(1) else {
            return Err(format!("Step {step}: “{text}” needs a step name after steps."));
        };
        if spec.step(other).is_none() {
            return Err(format!("Step {step}: “{text}” names a step, “{other}”, that does not exist."));
        }
        if other == step || !ancestors(spec, step).contains(other.as_str()) {
            return Err(format!("Step {step}: “{text}” reads step “{other}”, which does not run before it."));
        }
    }
    Ok(())
}

fn template_ok(spec: &WorkflowSpec, step: &str, text: &str, in_loop: bool) -> Result<(), String> {
    for path in template::paths(text).map_err(|e| format!("Step {step}: {e}"))? {
        path_ok(spec, step, &path, in_loop, &path.iter().map(show).collect::<Vec<_>>().join("."))?;
    }
    Ok(())
}

fn show(segment: &Segment) -> String {
    match segment {
        Segment::Key(k) => k.clone(),
        Segment::Index(i) => format!("[{i}]"),
    }
}

fn expr_ok(spec: &WorkflowSpec, step: &str, text: &str, what: &str) -> Result<(), String> {
    let expr = Expr::parse(text).map_err(|e| format!("Step {step}: the {what} is not valid: {e}"))?;
    for path in expr.paths() {
        let shown = path.iter().map(show).collect::<Vec<_>>().join(".");
        // A webhook filter reads the payload directly; a condition reads paths from the run.
        path_ok(spec, step, &path, false, &shown)?;
    }
    Ok(())
}

/// Check a workflow completely.
pub fn validate(spec: &WorkflowSpec) -> Result<(), String> {
    spec.validate().map_err(|e| e.to_string())?;
    for step in &spec.steps {
        let id = step.id.as_str();
        match &step.kind {
            StepKind::Schedule { when, timezone } => {
                next_fire(when, timezone.as_deref(), zorix_core::Timestamp(0))
                    .map_err(|e| format!("Step {id}: {e}"))?;
            }
            StepKind::Webhook { filter: Some(filter), .. } => {
                // The filter reads the request body: its paths start at `payload`.
                Expr::parse(filter).map_err(|e| format!("Step {id}: the filter is not valid: {e}"))?;
            }
            StepKind::Agent { prompt, .. } | StepKind::Team { prompt, .. } => template_ok(spec, id, prompt, false)?,
            StepKind::Condition { expr } => expr_ok(spec, id, expr, "condition")?,
            StepKind::Loop { over, prompt, .. } => {
                match template::paths(over) {
                    Ok(paths) if !paths.is_empty() => template_ok(spec, id, over, false)?,
                    Ok(_) => {
                        let path = parse_path(over).map_err(|e| format!("Step {id}: the list is not valid: {e}"))?;
                        path_ok(spec, id, &path, false, over)?;
                    }
                    Err(e) => return Err(format!("Step {id}: {e}")),
                }
                template_ok(spec, id, prompt, true)?;
            }
            StepKind::Approval { message, show, .. } => {
                template_ok(spec, id, message, false)?;
                for value in show {
                    let path = parse_path(value).map_err(|e| format!("Step {id}: “{value}” cannot be shown: {e}"))?;
                    path_ok(spec, id, &path, false, value)?;
                }
            }
            StepKind::Question { question, .. } => template_ok(spec, id, question, false)?,
            StepKind::Send { channel, text, .. } => {
                template_ok(spec, id, channel, false)?;
                template_ok(spec, id, text, false)?;
            }
            StepKind::Http { url, body, .. } => {
                template_ok(spec, id, url, false)?;
                if let Some(body) = body {
                    template_ok(spec, id, body, false)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use zorix_core::{AgentSlug, Ask, Edge, Service, Step, When, WorkflowSpec};

    use super::*;

    fn step(id: &str, kind: StepKind) -> Step {
        Step { id: id.into(), title: id.into(), x: 0, y: 0, kind }
    }

    fn agent(id: &str, prompt: &str) -> Step {
        step(
            id,
            StepKind::Agent {
                agent: AgentSlug::new("researcher").unwrap(),
                prompt: prompt.into(),
                version: None,
                model: None,
                max_cost_micros: None,
            },
        )
    }

    fn chain(steps: Vec<Step>) -> WorkflowSpec {
        let mut spec = WorkflowSpec::blank("x");
        let mut before = "start".to_string();
        for s in steps {
            spec.edges.push(Edge { from: before.clone(), to: s.id.clone(), label: None });
            before = s.id.clone();
            spec.steps.push(s);
        }
        spec
    }

    #[test]
    fn a_template_may_read_the_trigger_and_steps_before_it() {
        let spec = chain(vec![
            agent("a", "Look at {{trigger.payload.title}}"),
            agent("b", "Summarise {{steps.a.output.text}}"),
        ]);
        validate(&spec).unwrap();
    }

    #[test]
    fn a_template_cannot_read_a_step_that_does_not_exist_or_runs_later() {
        let spec = chain(vec![agent("a", "{{steps.zzz.output}}")]);
        assert!(validate(&spec).unwrap_err().contains("does not exist"));
        let spec = chain(vec![agent("a", "{{steps.b.output.text}}"), agent("b", "x")]);
        assert!(validate(&spec).unwrap_err().contains("does not run before"));
        let spec = chain(vec![agent("a", "{{steps.a.output}}")]);
        assert!(validate(&spec).unwrap_err().contains("does not run before"));
    }

    #[test]
    fn only_known_roots_and_loop_variables_are_allowed() {
        assert!(validate(&chain(vec![agent("a", "{{secrets.token}}")])).unwrap_err().contains("Start with"));
        assert!(validate(&chain(vec![agent("a", "{{item}}")])).unwrap_err().contains("only available in a loop"));
        let looped = step(
            "each",
            StepKind::Loop {
                over: "{{trigger.payload.items}}".into(),
                agent: AgentSlug::new("researcher").unwrap(),
                prompt: "Check {{item}} ({{index}})".into(),
            },
        );
        validate(&chain(vec![looped])).unwrap();
    }

    #[test]
    fn conditions_are_parsed_and_their_paths_checked() {
        let ok = chain(vec![
            agent("a", "x"),
            step("c", StepKind::Condition { expr: "steps.a.output.text contains 'high'".into() }),
        ]);
        // A condition needs labelled arrows to be a valid graph; add a target for each branch.
        let mut spec = ok;
        spec.steps.push(agent("y", "x"));
        spec.steps.push(agent("n", "x"));
        spec.edges.push(Edge { from: "c".into(), to: "y".into(), label: Some("yes".into()) });
        spec.edges.push(Edge { from: "c".into(), to: "n".into(), label: Some("no".into()) });
        validate(&spec).unwrap();

        let mut bad = spec.clone();
        bad.steps[2].kind = StepKind::Condition { expr: "steps.a.output ==".into() };
        assert!(validate(&bad).unwrap_err().contains("condition is not valid"));
        let mut missing = spec;
        missing.steps[2].kind = StepKind::Condition { expr: "steps.nope.output".into() };
        assert!(validate(&missing).unwrap_err().contains("does not exist"));
    }

    #[test]
    fn schedules_and_actions_are_checked() {
        let mut spec = WorkflowSpec::blank("x");
        spec.steps[0].kind = StepKind::Schedule { when: When::Cron { expr: "nonsense".into() }, timezone: None };
        assert!(validate(&spec).is_err());
        spec.steps[0].kind = StepKind::Schedule {
            when: When::Cron { expr: "0 8 * * 1-5".into() },
            timezone: Some("Europe/Lisbon".into()),
        };
        validate(&spec).unwrap();
        spec.steps[0].kind = StepKind::Schedule {
            when: When::Cron { expr: "0 8 * * 1-5".into() },
            timezone: Some("Nowhere/City".into()),
        };
        assert!(validate(&spec).unwrap_err().contains("time zone"));

        let send = step(
            "m",
            StepKind::Send {
                service: Service::Slack,
                connection: None,
                channel: "#ops".into(),
                text: "{{trigger.payload.nope".into(),
                ask: Ask::Always,
            },
        );
        assert!(validate(&chain(vec![send])).unwrap_err().contains("never closed"));
    }

    #[test]
    fn approval_values_are_paths_to_earlier_steps() {
        let approval = |show: &str| {
            step(
                "ok",
                StepKind::Approval {
                    message: "Go?".into(),
                    show: vec![show.into()],
                    timeout_ms: None,
                    then: zorix_core::Then::Refuse,
                    remind: None,
                },
            )
        };
        validate(&chain(vec![agent("a", "x"), approval("steps.a.output.text")])).unwrap();
        assert!(validate(&chain(vec![agent("a", "x"), approval("steps.q.output")])).is_err());
        assert!(validate(&chain(vec![agent("a", "x"), approval("run.cost_micros")])).is_ok());
    }
}
