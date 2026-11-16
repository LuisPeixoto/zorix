//! The four memory sources. Each one renders its part of the frozen recall (see [`crate::recall`]); none of them searches.

use std::sync::Arc;

use async_trait::async_trait;
use zorix_core::{ContextSegment, ContextSource, ExecutionView, Result};

use crate::doc::{ItemKind, render_chunk, render_exchange, render_facts, render_work};
use crate::recall::Recall;

/// Long-term facts and preferences a person approved.
pub struct LongTermSource(pub Arc<Recall>);

#[async_trait]
impl ContextSource for LongTermSource {
    fn name(&self) -> &str {
        "long_term"
    }

    async fn contribute(&self, view: &ExecutionView<'_>) -> Result<Vec<ContextSegment>> {
        let doc = self.0.get(view).await;
        Ok(render_facts(&doc.of(ItemKind::Fact)).into_iter().collect())
    }
}

/// What was asked and answered earlier in the same conversation.
pub struct ConversationSource(pub Arc<Recall>);

#[async_trait]
impl ContextSource for ConversationSource {
    fn name(&self) -> &str {
        "conversation"
    }

    async fn contribute(&self, view: &ExecutionView<'_>) -> Result<Vec<ContextSegment>> {
        let doc = self.0.get(view).await;
        let exchanges = doc.of(ItemKind::Exchange);
        let last = exchanges.len().saturating_sub(1);
        Ok(exchanges.iter().enumerate().map(|(i, item)| render_exchange(item, i == last)).collect())
    }
}

/// Pieces of indexed documents, with their sources.
pub struct KnowledgeSource(pub Arc<Recall>);

#[async_trait]
impl ContextSource for KnowledgeSource {
    fn name(&self) -> &str {
        "knowledge"
    }

    async fn contribute(&self, view: &ExecutionView<'_>) -> Result<Vec<ContextSegment>> {
        let doc = self.0.get(view).await;
        Ok(doc.of(ItemKind::Chunk).iter().map(|item| render_chunk(item)).collect())
    }
}

/// Records of past work.
pub struct OperationsSource(pub Arc<Recall>);

#[async_trait]
impl ContextSource for OperationsSource {
    fn name(&self) -> &str {
        "operations"
    }

    async fn contribute(&self, view: &ExecutionView<'_>) -> Result<Vec<ContextSegment>> {
        let doc = self.0.get(view).await;
        Ok(doc.of(ItemKind::PastWork).iter().map(|item| render_work(item)).collect())
    }
}
