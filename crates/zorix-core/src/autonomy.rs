//! How much a worker may do without asking (ADR-0039).
//!
//! The level is frozen into the run (on the agent binding) for the runs the workforce starts. The policy engine applies it; it can
//! make an answer stricter or, for a call that is held only by default, let it run without asking. It never relaxes a rule, a tool that
//! must be approved by a person, or an access pattern the worker listed to always ask.

use serde::{Deserialize, Serialize};

/// Five levels, from least to most free.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutonomyLevel {
    /// Proposes; a person acts. A call that changes anything does not run.
    Suggest,
    /// Acts after each approval. Every call that changes anything asks.
    Ask,
    /// Acts, then reports. A call held only by default runs, and a person is told.
    Notify,
    /// Acts within a weekly budget and a time limit.
    Budgeted,
    /// Acts and reports weekly.
    Full,
}

impl AutonomyLevel {
    /// Every level, least free first.
    pub const ALL: [AutonomyLevel; 5] = [
        AutonomyLevel::Suggest,
        AutonomyLevel::Ask,
        AutonomyLevel::Notify,
        AutonomyLevel::Budgeted,
        AutonomyLevel::Full,
    ];

    /// The word people read.
    pub fn label(self) -> &'static str {
        match self {
            AutonomyLevel::Suggest => "Suggest",
            AutonomyLevel::Ask => "Ask",
            AutonomyLevel::Notify => "Notify",
            AutonomyLevel::Budgeted => "Budgeted",
            AutonomyLevel::Full => "Full",
        }
    }

    /// The name used in storage and on the command line.
    pub fn as_str(self) -> &'static str {
        match self {
            AutonomyLevel::Suggest => "suggest",
            AutonomyLevel::Ask => "ask",
            AutonomyLevel::Notify => "notify",
            AutonomyLevel::Budgeted => "budgeted",
            AutonomyLevel::Full => "full",
        }
    }

    /// Read a level (any case).
    pub fn parse(text: &str) -> Result<Self, String> {
        let lower = text.trim().to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|l| l.as_str() == lower)
            .ok_or_else(|| format!("“{text}” is not a level: suggest, ask, notify, budgeted or full."))
    }

    /// Whether a call held only by default may run without asking at this level.
    pub fn runs_unattended(self) -> bool {
        self >= AutonomyLevel::Notify
    }
}

impl std::fmt::Display for AutonomyLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// The autonomy a run was started with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Autonomy {
    /// The level.
    pub level: AutonomyLevel,
    /// Access patterns (`repo:acme/web:write`) whose calls always ask, whatever the level.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub still_ask: Vec<String>,
}

impl Autonomy {
    /// A level with nothing extra that always asks.
    pub fn new(level: AutonomyLevel) -> Self {
        Self { level, still_ask: Vec::new() }
    }

    /// The stricter of two: the lower level, and every pattern that either asks about.
    pub fn stricter(&self, other: &Autonomy) -> Autonomy {
        let mut still_ask = self.still_ask.clone();
        for pattern in &other.still_ask {
            if !still_ask.contains(pattern) {
                still_ask.push(pattern.clone());
            }
        }
        Autonomy { level: self.level.min(other.level), still_ask }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_are_ordered_from_least_to_most_free() {
        assert!(AutonomyLevel::Suggest < AutonomyLevel::Ask);
        assert!(AutonomyLevel::Ask < AutonomyLevel::Notify);
        assert!(AutonomyLevel::Budgeted < AutonomyLevel::Full);
    }

    #[test]
    fn only_notify_and_above_run_unattended() {
        let flags: Vec<bool> = AutonomyLevel::ALL.iter().map(|l| l.runs_unattended()).collect();
        assert_eq!(flags, [false, false, true, true, true]);
    }

    #[test]
    fn levels_are_read_in_any_case_and_refuse_anything_else() {
        assert_eq!(AutonomyLevel::parse(" Budgeted ").unwrap(), AutonomyLevel::Budgeted);
        assert!(AutonomyLevel::parse("loose").is_err());
    }

    #[test]
    fn the_stricter_of_two_takes_the_lower_level_and_every_pattern() {
        let a = Autonomy { level: AutonomyLevel::Full, still_ask: vec!["db:a:write".into()] };
        let b = Autonomy { level: AutonomyLevel::Ask, still_ask: vec!["db:a:write".into(), "repo:x/y:write".into()] };
        let s = a.stricter(&b);
        assert_eq!(s.level, AutonomyLevel::Ask);
        assert_eq!(s.still_ask, vec!["db:a:write".to_string(), "repo:x/y:write".to_string()]);
    }

    #[test]
    fn a_level_serialises_as_its_name() {
        assert_eq!(serde_json::to_string(&AutonomyLevel::Budgeted).unwrap(), "\"budgeted\"");
        let a: Autonomy = serde_json::from_str(r#"{"level":"notify"}"#).unwrap();
        assert!(a.still_ask.is_empty());
    }
}
