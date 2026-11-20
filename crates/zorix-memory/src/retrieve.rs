//! Finding what is relevant to one execution: facts, earlier exchanges, document chunks and past work.
//!
//! Retrieval happens once per execution (ADR-0015). Keyword search always runs; vectors join in when an embeddings model is
//! configured; fusion and a relevance gate decide what is worth showing; each corpus gets a share of the token budget.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde_json::json;
use tracing::warn;
use zorix_core::{
    ChunkView, Clock, Corpus, Execution, ExecutionId, ExecutionRepository, ExecutionState, ExecutionView, ListFilter,
    MemoryFilter, MemoryItem, MemoryStore, RecallMode, Scope, TextHit, Timestamp, VectorHit, Visibility,
};

use crate::doc::{ItemKind, RecallDoc, RecallItem, chunk_id, exchange_id, fact_id, fact_meta, work_id};
use crate::embedder::Embedder;
use crate::fusion::{Fused, fuse, relevant};
use crate::text::{cut, date, query_text, terms, tokens};

/// How many hits each list keeps.
pub const LIST_SIZE: usize = 50;
/// Facts included whole, with no search, while there are no more than this many.
pub const SMALL_FACT_LIMIT: usize = 50;
/// A fact not confirmed for this long is labelled as possibly stale.
pub const STALE_DAYS: u64 = 180;
/// Chunks of one document that may be shown.
pub const CHUNKS_PER_DOCUMENT: usize = 3;
/// Exchanges of the conversation that may be shown.
pub const MAX_EXCHANGES: usize = 20;
/// Most characters of an answer or a prompt kept from an earlier exchange.
pub const EXCHANGE_CHARS: usize = 4_000;
/// Most characters of a past-work record shown.
pub const WORK_CHARS: usize = 1_200;

/// A share of the budget, and the most it may ever be.
#[derive(Clone, Copy, Debug)]
pub struct Share {
    /// Part of the budget, 0 to 1.
    pub fraction: f64,
    /// The most tokens, whatever the budget.
    pub cap: u64,
}

impl Share {
    /// Tokens this share allows out of `budget`.
    pub fn tokens(&self, budget: u64) -> u64 {
        ((budget as f64 * self.fraction) as u64).min(self.cap)
    }
}

/// How the budget is divided.
#[derive(Clone, Copy, Debug)]
pub struct Shares {
    /// Long-term facts.
    pub long_term: Share,
    /// The conversation.
    pub conversation: Share,
    /// Document chunks.
    pub knowledge: Share,
    /// Past work.
    pub operations: Share,
    /// Most chunks.
    pub max_chunks: usize,
    /// Most past-work records.
    pub max_records: usize,
}

impl Default for Shares {
    fn default() -> Self {
        Self {
            long_term: Share { fraction: 0.05, cap: 2_000 },
            conversation: Share { fraction: 0.20, cap: 16_000 },
            knowledge: Share { fraction: 0.15, cap: 8_000 },
            operations: Share { fraction: 0.05, cap: 2_000 },
            max_chunks: 8,
            max_records: 3,
        }
    }
}

/// The scopes in the order facts are shown: widest first.
fn scope_rank(scope: &Scope) -> u8 {
    match scope {
        Scope::Global => 0,
        Scope::Project(_) => 1,
        Scope::Conversation(_) => 2,
        Scope::Agent(_) => 3,
    }
}

/// What an execution can see.
pub fn visibility_of(execution: &Execution) -> Visibility {
    Visibility {
        tenant: execution.tenant.clone(),
        project: execution.workspace.as_ref().map(|p| p.to_string_lossy().into_owned()),
        conversation: execution.conversation.clone(),
        agent: execution.agent.as_ref().filter(|binding| binding.agent_memory).map(|binding| binding.agent),
    }
}

/// Finds what is relevant for an execution.
pub struct Retriever {
    store: Arc<dyn MemoryStore>,
    repository: Arc<dyn ExecutionRepository>,
    embedder: Option<Arc<Embedder>>,
    clock: Arc<dyn Clock>,
    shares: Shares,
}

impl Retriever {
    /// A retriever. Without an embedder, retrieval is by keywords alone.
    pub fn new(
        store: Arc<dyn MemoryStore>,
        repository: Arc<dyn ExecutionRepository>,
        embedder: Option<Arc<Embedder>>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self { store, repository, embedder, clock, shares: Shares::default() }
    }

    /// Divide the budget differently.
    pub fn with_shares(mut self, shares: Shares) -> Self {
        self.shares = shares;
        self
    }

    /// Everything relevant to the execution in `view`. Never fails: when something cannot be read, the document says so
    /// and holds what could be.
    pub async fn retrieve(&self, view: &ExecutionView<'_>) -> RecallDoc {
        let execution = view.execution;
        let options = execution.memory;
        let budget = view.context_budget.unwrap_or(crate::retrieve::DEFAULT_BUDGET);
        let factor = view.token_factor;
        let now = self.clock.now();
        let visibility = visibility_of(execution);
        let mut doc = RecallDoc::empty(String::new(), now);
        let mut problems: Vec<String> = Vec::new();

        // The conversation first: its last prompt helps to understand a short follow-up.
        let exchanges = if options.conversation {
            match self.exchanges(execution, budget, factor).await {
                Ok(found) => found,
                Err(error) => {
                    warn!(%error, "could not read the earlier executions of the conversation");
                    problems.push(format!("conversation unavailable: {error}"));
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        let previous = exchanges.last().and_then(|i| i.meta["prompt"].as_str());
        doc.query = query_text(&execution.request.prompt, previous);
        doc.terms = terms(&doc.query);

        // Facts: few of them are all included; many are searched.
        let facts = if options.long_term {
            match self.facts(&visibility, &doc.terms, budget, factor, now).await {
                Ok(found) => found,
                Err((found, error)) => {
                    warn!(%error, "could not read the long-term memory");
                    problems.push(format!("long-term memory unavailable: {error}"));
                    found
                }
            }
        } else {
            Vec::new()
        };
        // (The query vector is made at most once, and only when something will search with it.)
        let wants_vectors = options.knowledge == RecallMode::Auto || options.operations == RecallMode::Auto;
        let settings = self.store.settings(&execution.tenant).await.unwrap_or_default();
        let mut query_vector: Option<(String, Vec<f32>)> = None;
        if wants_vectors && let (Some(embedder), Some(model)) = (&self.embedder, settings.embedding_model.clone()) {
            match embedder.embed_query(&model, settings.embedding_dimensions, &doc.query).await {
                Ok((vector, used)) => {
                    doc.embedding_model = Some(model.clone());
                    doc.embedding_tokens = used;
                    query_vector = Some((model, vector));
                }
                Err(error) => problems.push(format!("embeddings unavailable: {error}")),
            }
        }
        let min_similarity =
            self.embedder.as_ref().map_or(crate::fusion::DEFAULT_MIN_SIMILARITY, |e| e.min_similarity());

        let chunks = if options.knowledge == RecallMode::Auto {
            match self.chunks(&visibility, &doc, query_vector.as_ref(), budget, factor, min_similarity).await {
                Ok(found) => found,
                Err(error) => {
                    warn!(%error, "could not search the documents");
                    problems.push(format!("documents unavailable: {error}"));
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        let works = if options.operations == RecallMode::Auto {
            match self.works(execution, &visibility, &doc, query_vector.as_ref(), budget, factor, min_similarity).await
            {
                Ok(found) => found,
                Err(error) => {
                    warn!(%error, "could not search past work");
                    problems.push(format!("past work unavailable: {error}"));
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

        doc.items.extend(facts);
        doc.items.extend(exchanges);
        doc.items.extend(chunks);
        doc.items.extend(works);
        if !problems.is_empty() {
            doc.degraded = Some(problems.join("; "));
        }
        doc
    }

    // ---- the conversation ----

    async fn exchanges(&self, execution: &Execution, budget: u64, factor: f64) -> zorix_core::Result<Vec<RecallItem>> {
        let Some(conversation) = execution.conversation.clone() else { return Ok(Vec::new()) };
        let filter = ListFilter {
            states: vec![ExecutionState::Completed, ExecutionState::Failed, ExecutionState::Cancelled],
            conversation: Some(conversation),
            limit: 100,
            ..ListFilter::default()
        };
        let earlier = self.repository.list(&execution.tenant, &filter).await?;
        let mut allowed = self.shares.conversation.tokens(budget);
        let mut kept: Vec<RecallItem> = Vec::new();
        // Newest first: the most recent exchanges are the ones that matter most, so they get the room.
        for past in
            earlier.iter().filter(|e| e.id != execution.id && e.created_at.millis() < execution.created_at.millis())
        {
            if kept.len() == MAX_EXCHANGES {
                break;
            }
            let (answer, state) = match past.state {
                ExecutionState::Completed => {
                    (past.result.as_ref().map(|r| r.text.clone()).unwrap_or_default(), "completed")
                }
                ExecutionState::Failed => {
                    (past.failure.as_ref().map(|f| f.message.clone()).unwrap_or_default(), "failed")
                }
                _ => (String::new(), "cancelled"),
            };
            let prompt = cut(&past.request.prompt, EXCHANGE_CHARS);
            let answer = cut(&zorix_core::redact_text(&answer), EXCHANGE_CHARS);
            let size = tokens(&prompt, factor) + tokens(&answer, factor);
            if size > allowed {
                break;
            }
            allowed -= size;
            kept.push(RecallItem {
                id: exchange_id(past.id),
                kind: ItemKind::Exchange,
                text: answer,
                label: date(past.created_at),
                meta: json!({"prompt": zorix_core::redact_text(&prompt), "state": state, "at": past.created_at.millis()}),
                score: 0.0,
                found_by: vec![],
                tokens: size,
            });
        }
        kept.reverse(); // oldest first, as it was said
        Ok(kept)
    }

    // ---- long-term facts ----

    /// The facts to show. On a failure, returns what was found with the error.
    async fn facts(
        &self,
        visibility: &Visibility,
        terms: &[String],
        budget: u64,
        factor: f64,
        now: Timestamp,
    ) -> Result<Vec<RecallItem>, (Vec<RecallItem>, zorix_core::Error)> {
        let allowed = self.shares.long_term.tokens(budget);
        let listed = self
            .store
            .list_memory(visibility, &MemoryFilter { limit: SMALL_FACT_LIMIT + 1, ..MemoryFilter::default() }, now)
            .await
            .map_err(|e| (Vec::new(), e))?;
        let mut items: Vec<(MemoryItem, f64, Vec<String>)> = if listed.len() <= SMALL_FACT_LIMIT {
            // Few facts: all of them, in a fixed order, which also keeps the prefix steady across executions.
            let mut all = listed;
            all.sort_by(|a, b| {
                scope_rank(&a.scope)
                    .cmp(&scope_rank(&b.scope))
                    .then(a.created_at.millis().cmp(&b.created_at.millis()))
                    .then(a.id.to_string().cmp(&b.id.to_string()))
            });
            all.into_iter().map(|i| (i, 0.0, vec![])).collect()
        } else {
            // Many: only the relevant ones, by keywords (vectors for memory are used when they exist).
            let keyword = self
                .store
                .text_search(visibility, Corpus::Memory, terms, LIST_SIZE)
                .await
                .map_err(|e| (Vec::new(), e))?;
            let fused = fuse(&keyword, &[]);
            let mut found = Vec::new();
            for hit in fused.iter().filter(|h| relevant(h, terms.len(), 1.0)) {
                if let Ok(id) = hit.id.parse()
                    && let Ok(Some(item)) = self.store.memory(&visibility.tenant, id).await
                    && item.in_force(now)
                {
                    found.push((item, hit.score, hit.found_by()));
                }
            }
            // Fresh before stale, then by relevance.
            found.sort_by(|a, b| {
                let stale = |i: &MemoryItem| days_since(i.confirmed_at, now) >= STALE_DAYS;
                stale(&a.0).cmp(&stale(&b.0)).then(b.1.total_cmp(&a.1))
            });
            found
        };
        let mut left = allowed;
        let mut out = Vec::new();
        for (item, score, found_by) in items.drain(..) {
            let stale_days = Some(days_since(item.confirmed_at, now)).filter(|d| *d >= STALE_DAYS);
            let size = tokens(&item.content, factor) + 24;
            if size > left {
                continue;
            }
            left -= size;
            out.push(RecallItem {
                id: fact_id(item.id),
                kind: ItemKind::Fact,
                text: zorix_core::redact_text(&item.content),
                label: item.scope.label(),
                meta: fact_meta(item.kind.as_str(), item.confirmed_at, stale_days),
                score,
                found_by,
                tokens: size,
            });
        }
        Ok(out)
    }

    // ---- documents ----

    async fn search(
        &self,
        visibility: &Visibility,
        corpus: Corpus,
        doc: &RecallDoc,
        vector: Option<&(String, Vec<f32>)>,
    ) -> zorix_core::Result<(Vec<TextHit>, Vec<VectorHit>)> {
        let keyword = self.store.text_search(visibility, corpus, &doc.terms, LIST_SIZE).await?;
        let by_vector = match vector {
            Some((model, query)) => self.store.vector_search(visibility, corpus, model, query, LIST_SIZE).await?,
            None => Vec::new(),
        };
        Ok((keyword, by_vector))
    }

    async fn chunks(
        &self,
        visibility: &Visibility,
        doc: &RecallDoc,
        vector: Option<&(String, Vec<f32>)>,
        budget: u64,
        factor: f64,
        min_similarity: f32,
    ) -> zorix_core::Result<Vec<RecallItem>> {
        let (keyword, by_vector) = self.search(visibility, Corpus::Knowledge, doc, vector).await?;
        let relevant_hits: Vec<Fused> =
            fuse(&keyword, &by_vector).into_iter().filter(|h| relevant(h, doc.terms.len(), min_similarity)).collect();
        if relevant_hits.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<i64> = relevant_hits.iter().filter_map(|h| h.id.parse().ok()).collect();
        let loaded: HashMap<i64, ChunkView> =
            self.store.chunks(&visibility.tenant, &ids).await?.into_iter().map(|c| (c.id, c)).collect();
        // Best first, at most three per document.
        let mut per_document: HashMap<(String, String), usize> = HashMap::new();
        let mut chosen: Vec<(ChunkView, &Fused)> = Vec::new();
        for hit in &relevant_hits {
            let Some(chunk) = hit.id.parse::<i64>().ok().and_then(|id| loaded.get(&id)) else { continue };
            let count = per_document.entry((chunk.root.to_string(), chunk.rel_path.clone())).or_default();
            if *count == CHUNKS_PER_DOCUMENT {
                continue;
            }
            *count += 1;
            chosen.push((chunk.clone(), hit));
        }
        // Neighbouring pieces of one document become one, so a passage is not cut in the middle.
        let merged = merge_neighbours(chosen);
        let mut left = self.shares.knowledge.tokens(budget);
        let mut out = Vec::new();
        for piece in merged {
            if out.len() == self.shares.max_chunks {
                break;
            }
            let size = tokens(&piece.text, factor) + 20;
            if size > left {
                continue;
            }
            left -= size;
            out.push(RecallItem { tokens: size, ..piece });
        }
        Ok(out)
    }

    // ---- past work ----

    #[allow(clippy::too_many_arguments)]
    async fn works(
        &self,
        execution: &Execution,
        visibility: &Visibility,
        doc: &RecallDoc,
        vector: Option<&(String, Vec<f32>)>,
        budget: u64,
        factor: f64,
        min_similarity: f32,
    ) -> zorix_core::Result<Vec<RecallItem>> {
        let (keyword, by_vector) = self.search(visibility, Corpus::Operations, doc, vector).await?;
        let mut left = self.shares.operations.tokens(budget);
        let mut out = Vec::new();
        for hit in fuse(&keyword, &by_vector).into_iter().filter(|h| relevant(h, doc.terms.len(), min_similarity)) {
            if out.len() == self.shares.max_records {
                break;
            }
            let Ok(id) = hit.id.parse::<ExecutionId>() else { continue };
            // This execution is not its own past; and the conversation's executions are already shown as exchanges.
            if id == execution.id {
                continue;
            }
            let Some(record) = self.store.operation(&visibility.tenant, id).await? else { continue };
            if execution.memory.conversation
                && execution.conversation.is_some()
                && record.conversation == execution.conversation
            {
                continue;
            }
            let text = cut(&zorix_core::redact_text(&record.text), WORK_CHARS);
            let size = tokens(&text, factor) + 30;
            if size > left {
                continue;
            }
            left -= size;
            out.push(RecallItem {
                id: work_id(id),
                kind: ItemKind::PastWork,
                text,
                label: date(record.finished_at),
                meta: json!({"state": record.state, "finished": date(record.finished_at)}),
                score: hit.score,
                found_by: hit.found_by(),
                tokens: size,
            });
        }
        Ok(out)
    }
}

/// The budget assumed when the model's window is not known (the same as the context builder's).
pub const DEFAULT_BUDGET: u64 = 24_000;

fn days_since(at: Timestamp, now: Timestamp) -> u64 {
    (now.millis().saturating_sub(at.millis()).max(0) as u64) / 86_400_000
}

/// Join pieces that are next to each other in a document into one, and keep the best score of the group.
fn merge_neighbours(chosen: Vec<(ChunkView, &Fused)>) -> Vec<RecallItem> {
    let mut groups: BTreeMap<(String, String), Vec<(ChunkView, &Fused)>> = BTreeMap::new();
    for (chunk, hit) in chosen {
        groups.entry((chunk.root.to_string(), chunk.rel_path.clone())).or_default().push((chunk, hit));
    }
    let mut items = Vec::new();
    for (_, mut pieces) in groups {
        pieces.sort_by_key(|(c, _)| c.ord);
        let mut run: Vec<(ChunkView, &Fused)> = Vec::new();
        let flush = |run: &mut Vec<(ChunkView, &Fused)>, items: &mut Vec<RecallItem>| {
            if run.is_empty() {
                return;
            }
            let first = &run[0].0;
            let last = &run[run.len() - 1].0;
            let mut view = first.clone();
            view.line_end = last.line_end;
            view.line_start = first.line_start;
            let text = run.iter().map(|(c, _)| c.text.as_str()).collect::<Vec<_>>().join("\n");
            let best =
                run.iter().map(|(_, h)| *h).max_by(|a, b| a.score.total_cmp(&b.score)).expect("a run is not empty");
            let mut found_by: Vec<String> = run.iter().flat_map(|(_, h)| h.found_by()).collect();
            found_by.sort();
            found_by.dedup();
            items.push(RecallItem {
                id: chunk_id(first.id),
                kind: ItemKind::Chunk,
                text: zorix_core::redact_text(&text),
                label: view.citation(),
                meta: json!({"heading": first.heading, "root": first.root_path, "chunks": run.iter().map(|(c, _)| c.id).collect::<Vec<_>>()}),
                score: best.score,
                found_by,
                tokens: 0,
            });
            run.clear();
        };
        for piece in pieces {
            if let Some(previous) = run.last()
                && piece.0.ord != previous.0.ord + 1
            {
                flush(&mut run, &mut items);
            }
            run.push(piece);
        }
        flush(&mut run, &mut items);
    }
    items.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
    items
}

/// What a search found.
pub struct Found {
    /// Relevant hits, best first.
    pub hits: Vec<Fused>,
    /// Why vectors did not take part, when they should have.
    pub degraded: Option<String>,
}

/// Keyword search, plus vectors when an embeddings model is set, fused and gated. Used by the search tools; retrieval for
/// the context does the same steps with its own budgets.
pub async fn hybrid_search(
    store: &dyn MemoryStore,
    embedder: Option<&Embedder>,
    visibility: &Visibility,
    corpus: Corpus,
    query: &str,
) -> zorix_core::Result<Found> {
    let query = query_text(query, None);
    let words = terms(&query);
    let keyword = store.text_search(visibility, corpus, &words, LIST_SIZE).await?;
    let mut by_vector = Vec::new();
    let mut degraded = None;
    let settings = store.settings(&visibility.tenant).await?;
    let mut min_similarity = crate::fusion::DEFAULT_MIN_SIMILARITY;
    if let (Some(embedder), Some(model)) = (embedder, settings.embedding_model) {
        min_similarity = embedder.min_similarity();
        match embedder.embed_query(&model, settings.embedding_dimensions, &query).await {
            Ok((vector, _)) => by_vector = store.vector_search(visibility, corpus, &model, &vector, LIST_SIZE).await?,
            Err(error) => degraded = Some(format!("embeddings unavailable: {error}")),
        }
    }
    let hits = fuse(&keyword, &by_vector).into_iter().filter(|h| relevant(h, words.len(), min_similarity)).collect();
    Ok(Found { hits, degraded })
}
