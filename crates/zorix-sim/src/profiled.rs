//! A wrapper that tells the runtime what the wrapped model can do (window, output limit, capabilities).

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use zorix_core::{ModelError, ModelGateway, ModelProfile, ModelRequest, ModelResponse};

/// Gives any model a [`ModelProfile`], so tests can exercise context budgeting.
pub struct Profiled<M> {
    inner: M,
    profile: ModelProfile,
}

impl<M: ModelGateway> Profiled<M> {
    /// Wrap a model.
    pub fn new(inner: M, profile: ModelProfile) -> Self {
        Self { inner, profile }
    }

    /// The wrapped model.
    pub fn inner(&self) -> &M {
        &self.inner
    }
}

#[async_trait]
impl<M: ModelGateway> ModelGateway for Profiled<M> {
    async fn generate(&self, request: ModelRequest, cancel: CancellationToken) -> Result<ModelResponse, ModelError> {
        self.inner.generate(request, cancel).await
    }

    fn describe(&self, _model: &str) -> Option<ModelProfile> {
        Some(self.profile)
    }
}
