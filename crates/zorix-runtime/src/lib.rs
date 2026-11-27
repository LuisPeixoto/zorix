//! ZORIX runtime: the execution engine, the loop, tools, policy, context and events.
//!
//! The contracts live in `zorix-core`; this crate holds the default implementations.

mod context;
mod engine;
mod events;
mod live;
mod memory;
mod policy;
mod retry;
mod run;
mod scheduler;
mod summary;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
mod tools;
mod validators;
mod watchdog;

pub use context::{
    DefaultContextBuilder, HistorySource, PREAMBLE, Placement, StaticSource, SystemSource, TaskSource, wrap_untrusted,
};
pub use engine::{Engine, EngineBuilder, EngineConfig};
pub use events::{EventBus, follow, is_final};
pub use live::LiveBus;
pub use memory::MemoryRepository;
pub use policy::{DefaultPolicy, Effect, Policy, Rule};
pub use retry::RetryPolicy;
pub use scheduler::LocalScheduler;
pub use summary::{ExecutionSummary, StepTiming, ToolSummary, summarize};
pub use tools::{CallError, InMemoryToolRegistry, ToolExecutor, cap_output};
pub use validators::{NonEmptyValidator, OutputSchemaValidator, parse_structured};
pub use zorix_core::{REDACTED, Secret, redact, redact_text};

#[cfg(test)]
mod memory_tests {
    use crate::MemoryRepository;

    crate::repository_contract_tests!(MemoryRepository::new());
}
