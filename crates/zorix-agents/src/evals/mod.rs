//! Evaluating agents (ADR-0028): cases with deterministic checks and an optional judge, run through the AgentRunner with
//! writes dropped and approvals refused, and compared across versions and models.

pub mod checks;
pub mod compare;
pub mod judge;
pub mod run;

pub use checks::run_checks;
pub use compare::{Change, CompareRow, compare};
pub use judge::judge;
pub use run::{EvalOptions, Evaluator, OnResult};
