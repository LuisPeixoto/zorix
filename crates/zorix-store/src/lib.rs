//! ZORIX store: SQLite persistence for executions, events and checkpoints.
//!
//! [`SqliteRepository`] implements `ExecutionRepository`; [`paths`] says where the data lives and keeps two programs
//! from using it at once.

pub mod agents;
mod calibration;
mod errors;
pub mod integrations;
pub mod memory;
pub mod org;
pub mod paths;
pub mod projects;
mod schema;
mod sqlite;
pub mod workflows;

pub use agents::SqliteAgentStore;
pub use calibration::SqliteCalibration;
pub use integrations::SqliteIntegrationStore;
pub use memory::SqliteMemoryStore;
pub use org::SqliteOrgStore;
pub use paths::{DataDirLock, HOME_VARIABLE, LockError, data_dir, database_path};
pub use projects::SqliteProjectStore;
pub use schema::LATEST as SCHEMA_VERSION;
pub use sqlite::SqliteRepository;
pub use workflows::SqliteWorkflowStore;

#[cfg(test)]
mod contract {
    use crate::SqliteRepository;

    zorix_runtime::repository_contract_tests!(SqliteRepository::open_in_memory().unwrap());
}
