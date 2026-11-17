//! Knowledge: indexing folders of documents so their text can be searched and cited (ADR-0017).

pub mod chunk;
pub mod extract;
#[cfg(feature = "pdf")]
pub mod pdf;
pub mod sync;
pub mod walk;

pub use sync::{Indexer, Phase, Progress, SyncReport};
