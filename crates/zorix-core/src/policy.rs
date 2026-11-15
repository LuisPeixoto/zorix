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

    /// Has the execution used up a limit? `None` means it may go on.
    fn check_limits(&self, execution: &Execution) -> Option<Failure>;
}
