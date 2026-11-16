//! Turning text into vectors for memory: one query at a time (with a short timeout) or many documents in batches.

use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use zorix_core::{EmbedPurpose, EmbeddingRequest, MemoryStore, ModelError, ModelErrorKind, ModelGateway};

use crate::Result;

use crate::fusion::DEFAULT_MIN_SIMILARITY;

/// How long a query may wait for its vector before retrieval goes on with keywords alone.
pub const QUERY_TIMEOUT: Duration = Duration::from_secs(5);
/// Texts per call when the model does not say.
pub const DEFAULT_BATCH: usize = 32;

/// Embeds text through a model gateway (the router, in the apps).
pub struct Embedder {
    gateway: Arc<dyn ModelGateway>,
    min_similarity: f32,
    query_timeout: Duration,
}

impl Embedder {
    /// An embedder over `gateway`.
    pub fn new(gateway: Arc<dyn ModelGateway>) -> Self {
        Self { gateway, min_similarity: DEFAULT_MIN_SIMILARITY, query_timeout: QUERY_TIMEOUT }
    }

    /// How close a vector-only hit must be to be trusted.
    pub fn with_min_similarity(mut self, min_similarity: f32) -> Self {
        self.min_similarity = min_similarity;
        self
    }

    /// How long a query may wait.
    pub fn with_query_timeout(mut self, timeout: Duration) -> Self {
        self.query_timeout = timeout;
        self
    }

    /// The similarity below which a vector-only hit is ignored.
    pub fn min_similarity(&self) -> f32 {
        self.min_similarity
    }

    /// The vector of a question and the tokens it used. Gives up after the query timeout.
    pub async fn embed_query(
        &self,
        model: &str,
        dimensions: Option<u32>,
        text: &str,
    ) -> std::result::Result<(Vec<f32>, u64), ModelError> {
        let request = EmbeddingRequest {
            model: model.into(),
            inputs: vec![text.to_string()],
            purpose: EmbedPurpose::Query,
            dimensions,
        };
        let call = self.gateway.embed(request, CancellationToken::new());
        let response = tokio::time::timeout(self.query_timeout, call).await.map_err(|_| {
            ModelError::provider(ModelErrorKind::Unavailable, "the embeddings model took too long to answer.")
        })??;
        let tokens = response.usage.input_tokens;
        response.vectors.into_iter().next().map(|v| (v, tokens)).ok_or_else(|| {
            ModelError::provider(ModelErrorKind::Unavailable, "the embeddings model returned no vector.")
        })
    }

    /// Embed `texts` (hash, text) as documents, in batches, and store the vectors under `model`. Returns how many were stored.
    /// Stops early, keeping what is stored, when `cancel` fires.
    pub async fn fill(
        &self,
        store: &dyn MemoryStore,
        model: &str,
        dimensions: Option<u32>,
        texts: Vec<(String, String)>,
        cancel: &CancellationToken,
    ) -> Result<usize> {
        let batch = self.gateway.embedding_profile(model).map_or(DEFAULT_BATCH, |p| p.max_batch).max(1);
        let mut stored = 0;
        for chunk in texts.chunks(batch) {
            if cancel.is_cancelled() {
                break;
            }
            let request = EmbeddingRequest {
                model: model.into(),
                inputs: chunk.iter().map(|(_, text)| text.clone()).collect(),
                purpose: EmbedPurpose::Document,
                dimensions,
            };
            let response = match self.gateway.embed(request, cancel.clone()).await {
                Ok(response) => response,
                Err(ModelError::Cancelled) => break,
                Err(error) => return Err(error.into()),
            };
            let vectors = chunk.iter().map(|(hash, _)| hash.clone()).zip(response.vectors).collect::<Vec<_>>();
            stored += vectors.len();
            store.put_vectors(model, vectors).await?;
        }
        Ok(stored)
    }
}
