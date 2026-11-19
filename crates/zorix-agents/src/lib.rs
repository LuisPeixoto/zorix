//! ZORIX's agents (ADR-0024 to ADR-0028): versioned definitions that create executions, their Markdown form, the skills they
//! load and the evaluations that compare them.

pub mod cards;
pub mod diff;
pub mod evals;
pub mod format;
pub mod runner;
pub mod skills;
pub mod starters;
pub mod yaml;

pub use cards::{AgentCard, cards};
pub use runner::{AgentAsk, ModelResolver, RunnerError, agent_grants, prepare};
pub use skills::{SkillError, SkillLibrary, skill_tools};
