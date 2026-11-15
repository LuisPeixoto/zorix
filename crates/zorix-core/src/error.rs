//! The error type shared by the contracts.
//!
//! Messages are full sentences a person can read; the interfaces show them as they are.

use crate::ids::ExecutionId;
use crate::state::InvalidTransition;

/// Anything that can go wrong when using the runtime through its contracts.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The lifecycle does not allow the requested step.
    #[error(transparent)]
    InvalidTransition(#[from] InvalidTransition),
    /// The request itself is not acceptable.
    #[error("Invalid request: {0}")]
    InvalidInput(String),
    /// There is no execution with this id (for this tenant).
    #[error("Execution {0} was not found.")]
    NotFound(ExecutionId),
    /// Somebody else changed the execution first. Reload it and try again.
    #[error("Execution {0} was changed by something else at the same time. Load it again and retry.")]
    Conflict(ExecutionId),
    /// Something that was asked for does not exist (an approval, a tool).
    #[error("{0}")]
    Missing(String),
    /// The storage layer failed.
    #[error("The storage failed: {0}")]
    Storage(String),
    /// A bug or an impossible situation inside the runtime.
    #[error("Internal error: {0}")]
    Internal(String),
}

/// A result with the runtime's [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;
