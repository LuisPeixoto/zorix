//! ZORIX runtime: the execution engine, the loop, tools, policy, context and events.
//!
//! The contracts live in `zorix-core`; this crate holds the default implementations.

mod context;
mod events;
mod memory;
mod policy;
mod redact;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
mod tools;

pub use context::{
    DefaultContextBuilder, HistorySource, PREAMBLE, StaticSource, SystemSource, TaskSource, wrap_untrusted,
};
pub use events::{EventBus, follow, is_final};
pub use memory::MemoryRepository;
pub use policy::{DefaultPolicy, Effect, Policy, Rule};
pub use redact::{REDACTED, Secret, redact, redact_text};
pub use tools::{CallError, InMemoryToolRegistry, ToolExecutor, cap_output};

#[cfg(test)]
mod memory_tests {
    use crate::MemoryRepository;

    crate::repository_contract_tests!(MemoryRepository::new());
}
