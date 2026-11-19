//! Two evaluation runs side by side, case by case.

use zorix_core::{EvalResult, EvalRun};

/// What changed for one case between two runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// It passed in both.
    StillPasses,
    /// It failed in both.
    StillFails,
    /// It failed before and passes now.
    Fixed,
    /// It passed before and fails now.
    Broke,
    /// The case is only in one of the runs.
    OnlyInOne,
}

/// One case in both runs.
#[derive(Clone, Debug, PartialEq)]
pub struct CompareRow {
    /// The case's name.
    pub case: String,
    /// What the first run found.
    pub before: Option<EvalResult>,
    /// What the second run found.
    pub after: Option<EvalResult>,
    /// What changed.
    pub change: Change,
    /// Second minus first cost, in millionths of a dollar.
    pub cost_delta_micros: i64,
    /// Second minus first steps.
    pub steps_delta: i64,
    /// Second minus first judge score, when both had one.
    pub score_delta: Option<i64>,
}

/// Compare two runs. Rows follow the cases of the first run, then those only the second has.
pub fn compare(before: &EvalRun, after: &EvalRun) -> Vec<CompareRow> {
    let find = |run: &'_ EvalRun, name: &str| run.results.iter().find(|r| r.case == name).cloned();
    let mut names: Vec<String> = before.results.iter().map(|r| r.case.clone()).collect();
    for r in &after.results {
        if !names.contains(&r.case) {
            names.push(r.case.clone());
        }
    }
    names
        .into_iter()
        .map(|case| {
            let (b, a) = (find(before, &case), find(after, &case));
            let change = match (&b, &a) {
                (Some(b), Some(a)) => match (b.passed, a.passed) {
                    (true, true) => Change::StillPasses,
                    (false, false) => Change::StillFails,
                    (false, true) => Change::Fixed,
                    (true, false) => Change::Broke,
                },
                _ => Change::OnlyInOne,
            };
            let cost = |r: &Option<EvalResult>| r.as_ref().map_or(0, |r| r.cost_micros as i64);
            let steps = |r: &Option<EvalResult>| r.as_ref().map_or(0, |r| i64::from(r.steps));
            let score_delta =
                match (b.as_ref().and_then(|r| r.judge.as_ref()), a.as_ref().and_then(|r| r.judge.as_ref())) {
                    (Some(x), Some(y)) => Some(i64::from(y.score) - i64::from(x.score)),
                    _ => None,
                };
            CompareRow {
                case,
                cost_delta_micros: cost(&a) - cost(&b),
                steps_delta: steps(&a) - steps(&b),
                score_delta,
                before: b,
                after: a,
                change,
            }
        })
        .collect()
}
