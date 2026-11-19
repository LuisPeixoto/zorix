//! ZORIX's agents (ADR-0024 to ADR-0028): versioned definitions that create executions, their Markdown form, the skills they
//! load and the evaluations that compare them.

pub mod diff;
pub mod format;
pub mod runner;
pub mod yaml;

pub use runner::{AgentAsk, ModelResolver, RunnerError, prepare};
