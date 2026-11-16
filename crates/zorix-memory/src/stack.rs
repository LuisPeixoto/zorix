//! The memory of an application, wired once and used by the command line and the desktop app alike.

use std::sync::Arc;

use zorix_core::{Clock, ExecutionRepository, MemoryStore, ModelGateway, SystemClock};
use zorix_runtime::{DefaultContextBuilder, EngineBuilder, Placement};

use crate::embedder::Embedder;
use crate::recall::Recall;
use crate::retrieve::{Retriever, Shares};
use crate::sources::{ConversationSource, KnowledgeSource, LongTermSource, OperationsSource};

/// Memory: a store, a way to find things in it, and the context sources that put them in front of the model.
pub struct MemoryStack {
    store: Arc<dyn MemoryStore>,
    recall: Arc<Recall>,
    embedder: Option<Arc<Embedder>>,
}

impl MemoryStack {
    /// A stack over `store`. `embeddings` is the gateway that turns text into vectors (the router, in the apps); without
    /// it, retrieval is by keywords alone.
    pub fn new(
        store: Arc<dyn MemoryStore>,
        repository: Arc<dyn ExecutionRepository>,
        embeddings: Option<Arc<dyn ModelGateway>>,
    ) -> Self {
        Self::with_clock(store, repository, embeddings, Arc::new(SystemClock), Shares::default())
    }

    /// The same with a chosen clock and budget shares (for tests).
    pub fn with_clock(
        store: Arc<dyn MemoryStore>,
        repository: Arc<dyn ExecutionRepository>,
        embeddings: Option<Arc<dyn ModelGateway>>,
        clock: Arc<dyn Clock>,
        shares: Shares,
    ) -> Self {
        let embedder = embeddings.map(|gateway| Arc::new(Embedder::new(gateway)));
        let retriever = Retriever::new(store.clone(), repository, embedder.clone(), clock.clone()).with_shares(shares);
        let recall = Arc::new(Recall::new(store.clone(), retriever, clock));
        Self { store, recall, embedder }
    }

    /// The store.
    pub fn store(&self) -> &Arc<dyn MemoryStore> {
        &self.store
    }

    /// The frozen recall (to invalidate after deleting memory).
    pub fn recall(&self) -> &Arc<Recall> {
        &self.recall
    }

    /// The embedder, when there is one.
    pub fn embedder(&self) -> Option<&Arc<Embedder>> {
        self.embedder.as_ref()
    }

    /// The default context builder with the four memory sources placed before the task, in this order: long-term facts,
    /// the conversation, knowledge and past work.
    pub fn context_builder(&self) -> DefaultContextBuilder {
        DefaultContextBuilder::new()
            .with_source_at(Placement::BeforeTask, Arc::new(LongTermSource(self.recall.clone())))
            .with_source_at(Placement::BeforeTask, Arc::new(ConversationSource(self.recall.clone())))
            .with_source_at(Placement::BeforeTask, Arc::new(KnowledgeSource(self.recall.clone())))
            .with_source_at(Placement::BeforeTask, Arc::new(OperationsSource(self.recall.clone())))
    }

    /// Put memory into an engine: its context builder.
    pub fn install(&self, builder: EngineBuilder) -> EngineBuilder {
        builder.context(Arc::new(self.context_builder()))
    }
}
