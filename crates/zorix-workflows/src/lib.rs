//! ZORIX workflows (ADR-0033 to ADR-0035): triggers, a graph of steps that start agents and actions, and people who decide.
//!
//! The model and its validation live in `zorix-core`; this crate runs them.

pub mod check;
pub mod engine;
pub mod expr;
pub mod graph;
pub mod host;
pub mod launcher;
pub mod runtime;
pub mod schedule;
pub mod service;
pub mod template;
pub mod webhook;

pub use engine::{Decision, StartRequest, Verdict, WorkflowEngine};
pub use host::{
    EngineFeed, ExecutionFeed, FinishedExecution, HostConfig, MISS_GRACE_MS, WorkflowHost, reset_webhook_token,
    signing_secret, webhook_token,
};
pub use launcher::{HubLauncher, Outcome, RunStart, StepLauncher, Target, ToolCall};
pub use runtime::{DEFAULT_WEBHOOK_ADDRESS, Workflows, parse_address};
pub use service::{
    Card, DiffLine, Figures, Health, Imported, Missing, StepHistory, Template, WorkflowFile, WorkflowService,
};
