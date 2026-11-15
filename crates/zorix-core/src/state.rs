//! The execution lifecycle.
//!
//! An execution can only change state through [`transition`]. It is a pure function, so the whole table is testable.
//! See ADR-0003 for the reasoning behind the states.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Where an execution is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionState {
    /// Stored, not yet asked to run.
    Created,
    /// Waiting for a worker.
    Queued,
    /// A worker is driving the loop (building context, validating, deciding the next step).
    Running,
    /// A model call is in flight.
    WaitingForModel,
    /// A tool call is in flight.
    WaitingForTool,
    /// Suspended until a person approves or denies a tool call.
    WaitingForApproval,
    /// Stopped on request; can be resumed.
    Paused,
    /// Finished with a result. Terminal.
    Completed,
    /// Finished with an error. Terminal.
    Failed,
    /// Stopped for good on request. Terminal.
    Cancelled,
}

impl ExecutionState {
    /// Every state, for exhaustive tests and listings.
    pub const ALL: [ExecutionState; 10] = [
        ExecutionState::Created,
        ExecutionState::Queued,
        ExecutionState::Running,
        ExecutionState::WaitingForModel,
        ExecutionState::WaitingForTool,
        ExecutionState::WaitingForApproval,
        ExecutionState::Paused,
        ExecutionState::Completed,
        ExecutionState::Failed,
        ExecutionState::Cancelled,
    ];

    /// True when nothing can happen to the execution any more.
    pub fn is_terminal(&self) -> bool {
        matches!(self, ExecutionState::Completed | ExecutionState::Failed | ExecutionState::Cancelled)
    }

    /// True when a worker is (or should be) actively working on it.
    pub fn is_active(&self) -> bool {
        matches!(self, ExecutionState::Running | ExecutionState::WaitingForModel | ExecutionState::WaitingForTool)
    }

    /// The name used in storage and in events.
    pub fn as_str(&self) -> &'static str {
        match self {
            ExecutionState::Created => "created",
            ExecutionState::Queued => "queued",
            ExecutionState::Running => "running",
            ExecutionState::WaitingForModel => "waiting_for_model",
            ExecutionState::WaitingForTool => "waiting_for_tool",
            ExecutionState::WaitingForApproval => "waiting_for_approval",
            ExecutionState::Paused => "paused",
            ExecutionState::Completed => "completed",
            ExecutionState::Failed => "failed",
            ExecutionState::Cancelled => "cancelled",
        }
    }
}

impl fmt::Display for ExecutionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ExecutionState {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        ExecutionState::ALL
            .into_iter()
            .find(|state| state.as_str() == text)
            .ok_or_else(|| format!("“{text}” is not an execution state."))
    }
}

/// What happened that may change the state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    /// The execution was submitted to the scheduler.
    Enqueue,
    /// A worker took it.
    Start,
    /// The loop is about to call the model.
    AwaitModel,
    /// The loop is about to run a tool.
    AwaitTool,
    /// A tool call needs a person's decision.
    AwaitApproval,
    /// The model call or the tool call finished.
    StepFinished,
    /// A person approved or denied; the execution goes back to the queue to continue.
    ApprovalResolved,
    /// A pause was requested and the loop reached a step boundary (or the execution was waiting in the queue).
    Pause,
    /// A paused execution was resumed.
    Resume,
    /// The process restarted and found the execution mid-work.
    Recover,
    /// The final answer was accepted.
    Complete,
    /// The execution failed.
    Fail,
    /// The execution was cancelled.
    Cancel,
}

impl Trigger {
    /// Every trigger, for exhaustive tests.
    pub const ALL: [Trigger; 13] = [
        Trigger::Enqueue,
        Trigger::Start,
        Trigger::AwaitModel,
        Trigger::AwaitTool,
        Trigger::AwaitApproval,
        Trigger::StepFinished,
        Trigger::ApprovalResolved,
        Trigger::Pause,
        Trigger::Resume,
        Trigger::Recover,
        Trigger::Complete,
        Trigger::Fail,
        Trigger::Cancel,
    ];
}

impl fmt::Display for Trigger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = serde_json::to_value(self).ok().and_then(|v| v.as_str().map(str::to_string));
        f.write_str(&name.unwrap_or_default())
    }
}

/// A change that the lifecycle does not allow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("An execution that is {from} cannot take the step “{trigger}”.")]
pub struct InvalidTransition {
    /// The state it was in.
    pub from: ExecutionState,
    /// What was attempted.
    pub trigger: Trigger,
}

/// The state that follows `trigger` in `from`, or an error when the lifecycle does not allow it.
pub fn transition(from: ExecutionState, trigger: Trigger) -> Result<ExecutionState, InvalidTransition> {
    use ExecutionState as S;
    use Trigger as T;
    let next = match (from, trigger) {
        (S::Created, T::Enqueue) => S::Queued,
        (S::Created, T::Cancel) => S::Cancelled,

        (S::Queued, T::Start) => S::Running,
        (S::Queued, T::Pause) => S::Paused,
        (S::Queued, T::Cancel) => S::Cancelled,

        (S::Running, T::AwaitModel) => S::WaitingForModel,
        (S::Running, T::AwaitTool) => S::WaitingForTool,
        (S::Running, T::AwaitApproval) => S::WaitingForApproval,
        (S::Running, T::Pause) => S::Paused,
        (S::Running, T::Recover) => S::Queued,
        (S::Running, T::Complete) => S::Completed,
        (S::Running, T::Fail) => S::Failed,
        (S::Running, T::Cancel) => S::Cancelled,

        (S::WaitingForModel, T::StepFinished) => S::Running,
        (S::WaitingForModel, T::Recover) => S::Queued,
        (S::WaitingForModel, T::Fail) => S::Failed,
        (S::WaitingForModel, T::Cancel) => S::Cancelled,

        (S::WaitingForTool, T::StepFinished) => S::Running,
        (S::WaitingForTool, T::AwaitApproval) => S::WaitingForApproval,
        (S::WaitingForTool, T::Recover) => S::Queued,
        (S::WaitingForTool, T::Fail) => S::Failed,
        (S::WaitingForTool, T::Cancel) => S::Cancelled,

        (S::WaitingForApproval, T::ApprovalResolved) => S::Queued,
        (S::WaitingForApproval, T::Fail) => S::Failed,
        (S::WaitingForApproval, T::Cancel) => S::Cancelled,

        (S::Paused, T::Resume) => S::Queued,
        (S::Paused, T::Cancel) => S::Cancelled,

        (from, trigger) => return Err(InvalidTransition { from, trigger }),
    };
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ExecutionState as S;
    use Trigger as T;

    /// The lifecycle written out a second time, as data, so a typo in the `match` shows up as a failing test.
    const ALLOWED: &[(ExecutionState, Trigger, ExecutionState)] = &[
        (S::Created, T::Enqueue, S::Queued),
        (S::Created, T::Cancel, S::Cancelled),
        (S::Queued, T::Start, S::Running),
        (S::Queued, T::Pause, S::Paused),
        (S::Queued, T::Cancel, S::Cancelled),
        (S::Running, T::AwaitModel, S::WaitingForModel),
        (S::Running, T::AwaitTool, S::WaitingForTool),
        (S::Running, T::AwaitApproval, S::WaitingForApproval),
        (S::Running, T::Pause, S::Paused),
        (S::Running, T::Recover, S::Queued),
        (S::Running, T::Complete, S::Completed),
        (S::Running, T::Fail, S::Failed),
        (S::Running, T::Cancel, S::Cancelled),
        (S::WaitingForModel, T::StepFinished, S::Running),
        (S::WaitingForModel, T::Recover, S::Queued),
        (S::WaitingForModel, T::Fail, S::Failed),
        (S::WaitingForModel, T::Cancel, S::Cancelled),
        (S::WaitingForTool, T::StepFinished, S::Running),
        (S::WaitingForTool, T::AwaitApproval, S::WaitingForApproval),
        (S::WaitingForTool, T::Recover, S::Queued),
        (S::WaitingForTool, T::Fail, S::Failed),
        (S::WaitingForTool, T::Cancel, S::Cancelled),
        (S::WaitingForApproval, T::ApprovalResolved, S::Queued),
        (S::WaitingForApproval, T::Fail, S::Failed),
        (S::WaitingForApproval, T::Cancel, S::Cancelled),
        (S::Paused, T::Resume, S::Queued),
        (S::Paused, T::Cancel, S::Cancelled),
    ];

    #[test]
    fn every_state_and_trigger_pair_behaves_as_documented() {
        for from in ExecutionState::ALL {
            for trigger in Trigger::ALL {
                let expected = ALLOWED.iter().find(|(f, t, _)| *f == from && *t == trigger).map(|(_, _, to)| *to);
                match (transition(from, trigger), expected) {
                    (Ok(got), Some(want)) => assert_eq!(got, want, "{from} + {trigger}"),
                    (Err(error), None) => assert_eq!(error, InvalidTransition { from, trigger }),
                    (got, want) => panic!("{from} + {trigger}: got {got:?}, documented {want:?}"),
                }
            }
        }
    }

    #[test]
    fn terminal_states_accept_nothing() {
        for from in ExecutionState::ALL.into_iter().filter(ExecutionState::is_terminal) {
            for trigger in Trigger::ALL {
                assert!(transition(from, trigger).is_err(), "{from} accepted {trigger}");
            }
        }
    }

    #[test]
    fn every_non_terminal_state_can_be_cancelled_except_the_ones_that_never_run() {
        for from in ExecutionState::ALL.into_iter().filter(|s| !s.is_terminal()) {
            assert_eq!(transition(from, Trigger::Cancel), Ok(S::Cancelled), "{from}");
        }
    }

    #[test]
    fn every_state_is_reachable_from_created() {
        let mut seen = vec![S::Created];
        let mut frontier = vec![S::Created];
        while let Some(state) = frontier.pop() {
            for trigger in Trigger::ALL {
                if let Ok(next) = transition(state, trigger)
                    && !seen.contains(&next)
                {
                    seen.push(next);
                    frontier.push(next);
                }
            }
        }
        assert_eq!(seen.len(), ExecutionState::ALL.len());
    }

    #[test]
    fn the_error_explains_itself() {
        let error = transition(S::Completed, T::Start).unwrap_err();
        assert_eq!(error.to_string(), "An execution that is completed cannot take the step “start”.");
    }

    #[test]
    fn states_round_trip_through_text_and_json() {
        for state in ExecutionState::ALL {
            assert_eq!(state.as_str().parse::<ExecutionState>().unwrap(), state);
            assert_eq!(serde_json::to_string(&state).unwrap(), format!("\"{}\"", state.as_str()));
        }
        assert!("bogus".parse::<ExecutionState>().is_err());
    }

    #[test]
    fn a_normal_run_with_one_tool_and_an_approval_is_a_legal_path() {
        let path = [
            T::Enqueue,
            T::Start,
            T::AwaitModel,
            T::StepFinished,
            T::AwaitTool,
            T::AwaitApproval,
            T::ApprovalResolved,
            T::Start,
            T::AwaitTool,
            T::StepFinished,
            T::AwaitModel,
            T::StepFinished,
            T::Complete,
        ];
        let end = path.iter().try_fold(S::Created, |state, trigger| transition(state, *trigger)).unwrap();
        assert_eq!(end, S::Completed);
    }
}
