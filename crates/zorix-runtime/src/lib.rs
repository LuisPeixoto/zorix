//! ZORIX runtime: the execution engine, the loop, tools, policy, context and events.
//!
//! The contracts live in `zorix-core`; this crate holds the default implementations.

mod memory;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use memory::MemoryRepository;

#[cfg(test)]
mod memory_tests {
    use crate::MemoryRepository;

    crate::repository_contract_tests!(MemoryRepository::new());
}
