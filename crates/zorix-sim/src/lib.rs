//! ZORIX sim: simulated models for tests, demos and development without a provider.

mod demo;
pub mod embed;
mod faulty;
mod profiled;
mod scripted;
mod usage;

pub use demo::DemoModel;
pub use faulty::Faulty;
pub use profiled::Profiled;
pub use scripted::{Failure, ScriptedCall, ScriptedModel, Turn};
