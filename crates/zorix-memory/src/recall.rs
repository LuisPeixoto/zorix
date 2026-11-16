//! The frozen recall of an execution (ADR-0015).
//!
//! The first time the context is built for an execution, memory is retrieved and stored as a snapshot. After that, every
//! turn — and every resume after a crash — reads the snapshot, so the same bytes are rendered each time and the provider's
//! cached prefix and reasoning state stay valid.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use parking_lot::Mutex;
use tracing::warn;
use zorix_core::{Clock, ExecutionId, ExecutionView, MemoryId, MemoryStore, RecallSnapshot};

use crate::doc::{ItemKind, RecallDoc};
use crate::retrieve::Retriever;

/// Snapshots kept in memory.
pub const CACHE_SIZE: usize = 64;

#[derive(Default)]
struct Cache {
    docs: HashMap<ExecutionId, Arc<RecallDoc>>,
    order: VecDeque<ExecutionId>,
}

impl Cache {
    fn get(&self, id: ExecutionId) -> Option<Arc<RecallDoc>> {
        self.docs.get(&id).cloned()
    }

    fn put(&mut self, id: ExecutionId, doc: Arc<RecallDoc>) {
        if self.docs.insert(id, doc).is_none() {
            self.order.push_back(id);
            while self.order.len() > CACHE_SIZE {
                if let Some(old) = self.order.pop_front() {
                    self.docs.remove(&old);
                }
            }
        }
    }
}

/// Retrieves memory for an execution once and hands out the same document afterwards.
pub struct Recall {
    store: Arc<dyn MemoryStore>,
    retriever: Retriever,
    clock: Arc<dyn Clock>,
    cache: Mutex<Cache>,
    locks: Mutex<HashMap<ExecutionId, Arc<tokio::sync::Mutex<()>>>>,
}

impl Recall {
    /// A recall over a store and a retriever.
    pub fn new(store: Arc<dyn MemoryStore>, retriever: Retriever, clock: Arc<dyn Clock>) -> Self {
        Self { store, retriever, clock, cache: Mutex::new(Cache::default()), locks: Mutex::new(HashMap::new()) }
    }

    /// The document of the execution in `view`: from memory, from the stored snapshot, or retrieved now (and stored).
    pub async fn get(&self, view: &ExecutionView<'_>) -> Arc<RecallDoc> {
        let execution = view.execution;
        let id = execution.id;
        let options = execution.memory;
        if !options.conversation
            && !options.long_term
            && options.knowledge == zorix_core::RecallMode::Off
            && options.operations == zorix_core::RecallMode::Off
        {
            return Arc::new(RecallDoc::empty(String::new(), self.clock.now()));
        }
        if let Some(doc) = self.cache.lock().get(id) {
            return doc;
        }
        // One retrieval per execution, even if two builds ask at once.
        let lock = self.locks.lock().entry(id).or_default().clone();
        let _held = lock.lock().await;
        if let Some(doc) = self.cache.lock().get(id) {
            return doc;
        }
        let doc = match self.store.snapshot(id).await {
            Ok(Some(snapshot)) => match serde_json::from_value::<RecallDoc>(snapshot.doc) {
                Ok(doc) => Arc::new(doc),
                Err(error) => {
                    warn!(%error, "a stored recall snapshot could not be read; retrieving again");
                    self.create(view).await
                }
            },
            Ok(None) => self.create(view).await,
            Err(error) => {
                warn!(%error, "could not read the recall snapshot; retrieving without saving");
                let mut doc = self.retriever.retrieve(view).await;
                doc.degraded = Some(format!("memory could not be saved: {error}"));
                Arc::new(doc)
            }
        };
        self.cache.lock().put(id, doc.clone());
        self.locks.lock().remove(&id);
        doc
    }

    async fn create(&self, view: &ExecutionView<'_>) -> Arc<RecallDoc> {
        let execution = view.execution;
        let mut doc = self.retriever.retrieve(view).await;
        let snapshot = RecallSnapshot {
            execution_id: execution.id,
            tenant: execution.tenant.clone(),
            created_at: self.clock.now(),
            items: doc.snapshot_keys(),
            doc: serde_json::to_value(&doc).unwrap_or(serde_json::Value::Null),
        };
        match self.store.save_snapshot(snapshot).await {
            Ok(stored) => match serde_json::from_value::<RecallDoc>(stored.doc) {
                // Another process or an earlier run may have saved first: the stored one is the one that stands.
                Ok(winner) if winner != doc => return Arc::new(winner),
                _ => {}
            },
            Err(error) => {
                warn!(%error, "could not save the recall snapshot");
                doc.degraded = Some(match doc.degraded.take() {
                    Some(earlier) => format!("{earlier}; the snapshot could not be saved: {error}"),
                    None => format!("the snapshot could not be saved: {error}"),
                });
                return Arc::new(doc);
            }
        }
        // It stands: note that these facts were used.
        let used: Vec<MemoryId> = doc
            .of(ItemKind::Fact)
            .iter()
            .filter_map(|i| i.id.strip_prefix("m:").and_then(|id| id.parse().ok()))
            .collect();
        if !used.is_empty()
            && let Err(error) = self.store.mark_used(&execution.tenant, &used, self.clock.now()).await
        {
            warn!(%error, "could not note which memories were used");
        }
        Arc::new(doc)
    }

    /// Forget what is held in memory, so snapshots are read again from the store. Call it after deleting memory.
    pub fn invalidate_all(&self) {
        let mut cache = self.cache.lock();
        cache.docs.clear();
        cache.order.clear();
    }

    /// Forget one execution's snapshot held in memory.
    pub fn invalidate(&self, id: ExecutionId) {
        let mut cache = self.cache.lock();
        cache.docs.remove(&id);
        cache.order.retain(|x| *x != id);
    }
}
