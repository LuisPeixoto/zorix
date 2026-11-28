//! The policy engine: the one place that decides whether a call may run (ADR-0006).

use serde_json::Value;

use crate::execution::{Execution, Failure};
use crate::tool::ToolSpec;

/// What is being decided.
#[derive(Clone, Copy, Debug)]
pub struct PolicyRequest<'a> {
    /// The execution asking.
    pub execution: &'a Execution,
    /// The tool it wants to run.
    pub tool: &'a ToolSpec,
    /// The exact arguments.
    pub arguments: &'a Value,
}

/// The answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyDecision {
    /// Run it.
    Allow,
    /// Run it without asking, although the default would have held it, because of the worker's level (ADR-0039). The run
    /// records the call, its arguments and the level.
    AllowUnattended {
        /// A sentence about why.
        reason: String,
    },
    /// Do not run it: a worker at the Suggest level proposes, a person acts. The call is recorded as a suggestion.
    Suggest {
        /// A sentence about why.
        reason: String,
    },
    /// Do not run it. The reason goes back to the model as the tool result.
    Deny {
        /// A sentence about why.
        reason: String,
    },
    /// Suspend until a person decides.
    RequireApproval {
        /// A sentence about why.
        reason: String,
    },
}

/// Decides, at the moment of execution, what is allowed.
///
/// The brief lists a permission engine and a policy engine. They are one contract here: grants are checked by the same
/// code that applies rules and limits, so there is no way to run a call that only one of them looked at.
pub trait PolicyEngine: Send + Sync {
    /// May this call run now?
    fn evaluate_tool_call(&self, request: &PolicyRequest<'_>) -> PolicyDecision;

    /// The reason a call is refused whatever else is true of it: a forbidden action, a rule that blocks the tool, a tool that
    /// reaches outside this computer where that is not allowed, or a permission that `grants` lack. `None` when none of those hold.
    ///
    /// [`evaluate_tool_call`](Self::evaluate_tool_call) starts with this, and so does anything that runs a tool without an execution
    /// (a workflow action), so that no way of running a call skips the rules. Asking a person is not part of it: whoever runs the
    /// call decides how to ask.
    fn standing_denial(&self, tool: &ToolSpec, grants: &crate::Grants) -> Option<String>;

    /// Has the execution used up a limit? `None` means it may go on.
    fn check_limits(&self, execution: &Execution) -> Option<Failure>;
}
