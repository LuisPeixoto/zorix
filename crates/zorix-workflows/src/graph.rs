//! Which steps are ready, which are skipped, and which steps run before which (ADR-0033). Pure.

use std::collections::{BTreeMap, BTreeSet};

use zorix_core::{StepRun, StepState, WorkflowSpec};

/// What a step that has not started can do next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readiness {
    /// An input has not settled.
    NotYet,
    /// Every input settled and at least one carries the run to this step.
    Ready,
    /// Every input settled and none does: the step is on a branch that was not taken.
    Skip,
}

/// Whether the arrow `from → to` carries the run onward: its source is done and, when the arrow is labelled, the source took
/// that branch.
fn carries(steps: &BTreeMap<String, StepRun>, from: &str, label: Option<&str>) -> bool {
    let Some(run) = steps.get(from) else { return false };
    if run.state != StepState::Done {
        return false;
    }
    match label {
        None => true,
        Some(label) => run.note.as_deref() == Some(label),
    }
}

/// The readiness of the step `id`.
pub fn readiness(spec: &WorkflowSpec, steps: &BTreeMap<String, StepRun>, id: &str) -> Readiness {
    let inputs = spec.inputs(id);
    if inputs.is_empty() {
        return Readiness::NotYet;
    }
    let mut live = false;
    for edge in inputs {
        let Some(source) = steps.get(&edge.from) else { return Readiness::NotYet };
        if !source.state.is_settled() {
            return Readiness::NotYet;
        }
        if carries(steps, &edge.from, edge.label.as_deref()) {
            live = true;
        }
    }
    if live { Readiness::Ready } else { Readiness::Skip }
}

/// Every step that runs before `id` on some path.
pub fn ancestors(spec: &WorkflowSpec, id: &str) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut frontier = vec![id.to_string()];
    while let Some(next) = frontier.pop() {
        for edge in spec.inputs(&next) {
            if seen.insert(edge.from.clone()) {
                frontier.push(edge.from.clone());
            }
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use zorix_core::{AgentSlug, Edge, Step, StepKind};

    use super::*;

    fn agent(id: &str) -> Step {
        Step {
            id: id.into(),
            title: id.into(),
            x: 0,
            y: 0,
            kind: StepKind::Agent {
                agent: AgentSlug::new("a").unwrap(),
                prompt: "p".into(),
                version: None,
                model: None,
                max_cost_micros: None,
            },
        }
    }

    fn condition(id: &str) -> Step {
        Step { id: id.into(), title: id.into(), x: 0, y: 0, kind: StepKind::Condition { expr: "x".into() } }
    }

    fn edge(from: &str, to: &str, label: Option<&str>) -> Edge {
        Edge { from: from.into(), to: to.into(), label: label.map(String::from) }
    }

    /// start → c ─yes→ a ─┐
    ///             └no→ b ─┴→ join
    fn spec() -> WorkflowSpec {
        let mut spec = WorkflowSpec::blank("x");
        spec.steps.extend([condition("c"), agent("a"), agent("b"), agent("join")]);
        spec.edges.extend([
            edge("start", "c", None),
            edge("c", "a", Some("yes")),
            edge("c", "b", Some("no")),
            edge("a", "join", None),
            edge("b", "join", None),
        ]);
        spec.validate().unwrap();
        spec
    }

    fn state(spec: &WorkflowSpec, states: &[(&str, StepState, Option<&str>)]) -> BTreeMap<String, StepRun> {
        let mut map: BTreeMap<String, StepRun> =
            spec.steps.iter().map(|s| (s.id.clone(), StepRun::default())).collect();
        for (id, state, note) in states {
            let run = map.get_mut(*id).unwrap();
            run.state = *state;
            run.note = note.map(String::from);
        }
        map
    }

    #[test]
    fn a_step_waits_for_every_input_to_settle() {
        let spec = spec();
        let steps = state(&spec, &[("start", StepState::Done, None)]);
        assert_eq!(readiness(&spec, &steps, "c"), Readiness::Ready);
        assert_eq!(readiness(&spec, &steps, "a"), Readiness::NotYet);
    }

    #[test]
    fn the_branch_not_taken_is_skipped_and_the_skip_passes_on() {
        let spec = spec();
        let mut steps = state(&spec, &[("start", StepState::Done, None), ("c", StepState::Done, Some("yes"))]);
        assert_eq!(readiness(&spec, &steps, "a"), Readiness::Ready);
        assert_eq!(readiness(&spec, &steps, "b"), Readiness::Skip);
        steps.get_mut("b").unwrap().state = StepState::Skipped;
        // The join waits for a, which has not finished, even though b was skipped.
        assert_eq!(readiness(&spec, &steps, "join"), Readiness::NotYet);
        steps.get_mut("a").unwrap().state = StepState::Done;
        assert_eq!(readiness(&spec, &steps, "join"), Readiness::Ready);
    }

    #[test]
    fn a_join_whose_inputs_were_all_skipped_is_skipped() {
        let spec = spec();
        let steps = state(
            &spec,
            &[
                ("start", StepState::Done, None),
                ("c", StepState::Skipped, None),
                ("a", StepState::Skipped, None),
                ("b", StepState::Skipped, None),
            ],
        );
        assert_eq!(readiness(&spec, &steps, "join"), Readiness::Skip);
    }

    #[test]
    fn parallel_branches_join_when_both_are_done() {
        let mut spec = WorkflowSpec::blank("x");
        spec.steps.extend([agent("a"), agent("b"), agent("join")]);
        spec.edges.extend([
            edge("start", "a", None),
            edge("start", "b", None),
            edge("a", "join", None),
            edge("b", "join", None),
        ]);
        spec.validate().unwrap();
        let steps = state(
            &spec,
            &[("start", StepState::Done, None), ("a", StepState::Done, None), ("b", StepState::Running, None)],
        );
        assert_eq!(readiness(&spec, &steps, "join"), Readiness::NotYet);
        let steps = state(
            &spec,
            &[("start", StepState::Done, None), ("a", StepState::Done, None), ("b", StepState::Done, None)],
        );
        assert_eq!(readiness(&spec, &steps, "join"), Readiness::Ready);
    }

    #[test]
    fn ancestors_are_every_step_before() {
        let spec = spec();
        let before: Vec<String> = ancestors(&spec, "join").into_iter().collect();
        assert_eq!(before, vec!["a", "b", "c", "start"]);
        assert!(ancestors(&spec, "start").is_empty());
    }
}
