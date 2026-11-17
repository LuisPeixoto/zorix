//! ZORIX memory: what is remembered between executions, and how it reaches the context (ADR-0013 to ADR-0017).
//!
//! Four kinds feed the context as sources: the **conversation** (earlier executions of the same conversation),
//! **long-term** facts a person approved, **knowledge** from indexed documents and **operational** records of past work.
//! They are retrieved once per execution and frozen (ADR-0015), so every turn and every resume sends the same bytes.

pub mod doc;
pub mod embedder;
pub mod fusion;
pub mod ops;
pub mod recall;
pub mod retrieve;
pub mod sources;
pub mod stack;
pub mod text;
pub mod tools;

/// What can go wrong while memory is read or filled.
#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    /// The store failed.
    #[error(transparent)]
    Store(#[from] zorix_core::Error),
    /// The embeddings model failed.
    #[error("{0}")]
    Model(#[from] zorix_core::ModelError),
}

/// A result with [`MemoryError`].
pub type Result<T, E = MemoryError> = std::result::Result<T, E>;

pub use doc::{ItemKind, RecallDoc, RecallItem};
pub use embedder::Embedder;
pub use ops::OperationRecorder;
pub use recall::Recall;
pub use sources::{ConversationSource, KnowledgeSource, LongTermSource, OperationsSource};
pub use stack::MemoryStack;
pub use tools::MemoryTools;
