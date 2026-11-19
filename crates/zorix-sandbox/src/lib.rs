//! ZORIX's code sandbox (ADR-0022): JavaScript and Python run in WebAssembly, so the same module behaves the same on every
//! system. A run has no network, no environment variables and no clock beyond WASI's; it sees a scratch folder that is deleted
//! afterwards and, when a person allows it, a folder mounted read-only. Memory, time and output are limited.

pub mod install;
pub mod runner;
pub mod tool;

pub use runner::{Language, Limits, Outcome, Run, Sandbox, SandboxError, StopReason};
pub use tool::sandbox_tools;
