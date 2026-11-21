//! ZORIX workflows (ADR-0033 to ADR-0035): triggers, a graph of steps that start agents and actions, and people who decide.
//!
//! The model and its validation live in `zorix-core`; this crate runs them.

pub mod check;
pub mod engine;
pub mod expr;
pub mod graph;
pub mod launcher;
pub mod schedule;
pub mod template;

pub use engine::{Decision, StartRequest, Verdict, WorkflowEngine};
pub use launcher::{HubLauncher, Outcome, RunStart, StepLauncher, Target, ToolCall};
