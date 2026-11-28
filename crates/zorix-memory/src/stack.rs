//! The memory of an application, wired once and used by the command line and the desktop app alike.

use std::sync::Arc;

use zorix_core::{Clock, ExecutionRepository, MemoryStore, ModelGateway, SystemClock};
use zorix_runtime::{DefaultContextBuilder, EngineBuilder, Placement};

use crate::embedder::Embedder;
use crate::recall::Recall;
use crate::retrieve::{Retriever, Shares};
use crate::sources::{ConversationSource, KnowledgeSource, LongTermSource, OperationsSource};
use crate::tools::MemoryTools;

/// Memory: a store, a way to find things in it, and the context sources that put them in front of the model.
pub struct MemoryStack {
    store: Arc<dyn MemoryStore>,
    repository: Arc<dyn ExecutionRepository>,
    clock: Arc<dyn Clock>,
    recall: Arc<Recall>,
    embedder: Option<Arc<Embedder>>,
    recorder: parking_lot::Mutex<Option<Arc<crate::ops::OperationRecorder>>>,
    /// What reads scanned PDFs, when reading scans is on.
    ocr: parking_lot::Mutex<Option<Arc<dyn crate::index::PageReader>>>,
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
        let retriever =
            Retriever::new(store.clone(), repository.clone(), embedder.clone(), clock.clone()).with_shares(shares);
        let recall = Arc::new(Recall::new(store.clone(), retriever, clock.clone()));
        Self {
            store,
            repository,
            clock,
            recall,
            embedder,
            recorder: parking_lot::Mutex::new(None),
            ocr: parking_lot::Mutex::new(None),
        }
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

    /// The tools through which a model reads and proposes memory.
    pub fn tools(&self) -> MemoryTools {
        MemoryTools::new(self.store.clone(), self.repository.clone(), self.embedder.clone(), self.clock.clone())
    }

    /// Register the memory tools. Call it before building the engine, with the registry the engine will use.
    pub fn register_tools(&self, registry: &dyn zorix_core::ToolRegistry) -> zorix_core::Result<()> {
        self.tools().register(registry)
    }

    /// Make vectors for the facts and past-work records that have none, with the model chosen in the settings. Returns how
    /// many were made. Without an embeddings model this does nothing.
    pub async fn embed_missing(
        &self,
        tenant: &zorix_core::TenantId,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> crate::Result<usize> {
        let Some(embedder) = &self.embedder else { return Ok(0) };
        let settings = self.store.settings(tenant).await?;
        let Some(model) = settings.embedding_model else { return Ok(0) };
        let mut total = 0;
        while !cancel.is_cancelled() {
            let batch = self.store.texts_without_vector(tenant, &model, 64).await?;
            if batch.is_empty() {
                break;
            }
            let stored = embedder.fill(&*self.store, &model, settings.embedding_dimensions, batch, cancel).await?;
            if stored == 0 {
                break;
            }
            total += stored;
        }
        Ok(total)
    }

    /// Start over with the embeddings model in the settings: sync every folder (which embeds what lacks a vector), embed the
    /// facts and past work, then delete the vectors of any other model. Returns what each folder did and how many facts and
    /// records were embedded.
    pub async fn reembed(
        &self,
        tenant: &zorix_core::TenantId,
        progress: &(dyn Fn(&zorix_core::KnowledgeRoot, crate::index::Progress) + Send + Sync),
        cancel: &tokio_util::sync::CancellationToken,
    ) -> crate::Result<(Vec<(zorix_core::KnowledgeRoot, crate::index::SyncReport)>, usize)> {
        let indexer = self.indexer();
        let mut reports = Vec::new();
        for root in self.store.all_roots(tenant).await? {
            if cancel.is_cancelled() {
                break;
            }
            if !root.enabled {
                continue;
            }
            let report = indexer.sync(&root, &|p| progress(&root, p), cancel).await?;
            reports.push((root, report));
        }
        let memory = self.embed_missing(tenant, cancel).await?;
        if !cancel.is_cancelled() {
            let settings = self.store.settings(tenant).await?;
            self.store.gc_vectors(settings.embedding_model.as_deref()).await?;
        }
        Ok((reports, memory))
    }

    /// The indexer for knowledge folders.
    pub fn indexer(&self) -> crate::index::Indexer {
        crate::index::Indexer::new(self.store.clone(), self.embedder.clone()).with_ocr(self.ocr.lock().clone())
    }

    /// Read the files a person attached to a run, with scanned PDFs read too when reading scans is on (ADR-0049). Blocking: run it off the async threads.
    pub fn read_attachments(
        &self,
        paths: &[std::path::PathBuf],
    ) -> Result<Vec<zorix_core::Attachment>, crate::attach::AttachError> {
        crate::attach::read_attachments(paths, self.ocr.lock().clone())
    }

    /// Whether scanned PDFs are read now.
    pub fn has_ocr(&self) -> bool {
        self.ocr.lock().is_some()
    }

    /// Read scanned PDFs with this from now on (or stop reading them, with `None`).
    pub fn set_ocr(&self, reader: Option<Arc<dyn crate::index::PageReader>>) {
        *self.ocr.lock() = reader;
    }

    /// Put memory into an engine: its context builder, and the recorder that keeps a record of every execution that ends.
    /// Call it inside a Tokio runtime (the recorder starts a background task).
    pub fn install(&self, builder: EngineBuilder) -> EngineBuilder {
        let recorder =
            crate::ops::OperationRecorder::start(self.store.clone(), self.repository.clone(), self.embedder.clone());
        *self.recorder.lock() = Some(recorder.clone());
        builder.context(Arc::new(self.context_builder())).publisher(recorder)
    }

    /// Write the records of finished executions that have none. Returns how many were written.
    pub async fn backfill_operations(&self) -> zorix_core::Result<usize> {
        let recorder = self.recorder.lock().clone();
        match recorder {
            Some(recorder) => recorder.backfill(200).await,
            None => Ok(0),
        }
    }
}
