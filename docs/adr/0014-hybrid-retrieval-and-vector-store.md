# ADR-0014: Hybrid retrieval with a relevance gate; vectors in SQLite blobs

Status: accepted (Phase 3)

## Context

Keyword search misses paraphrases; embeddings return a nearest neighbour even when nothing is relevant, and need a model
that may not exist. A vector database would be a second moving part for a corpus of thousands of chunks.

## Decision

- Retrieval is **FTS5 BM25 always, plus embeddings when an embeddings model is configured**, merged by reciprocal rank
  fusion (k = 60).
- A **relevance gate** follows the fusion: a hit is kept only if it is in both lists, or in the keyword top 5 with enough
  distinct matching terms, or in the vector top 5 above the model's minimum similarity. An unrelated corpus yields nothing.
- Queries are built from sanitised terms; raw text never reaches `MATCH`.
- Embedding the query has a short timeout; on failure retrieval runs on keywords and the snapshot records why it was degraded.
- Vectors are L2-normalised f32 blobs keyed by `(model, text_hash)`, scanned by brute force behind one store method, with
  limits (200k vectors per tenant and model, a warning at 50k).
- One embeddings model per tenant. `Router::embed` accepts exactly one explicit model, **never falls back**, and respects
  `local_only`. Changing the model re-embeds; old vectors are collected afterwards.
- Cloud embeddings are opt-in and say so: they send document text to the provider.

## Consequences

- Works offline and with no setup; gets better with a local embeddings model.
- Brute force is fine at desktop scale and replaceable (`sqlite-vec`, quantisation) without touching contracts.
- The gate's thresholds need tuning on real data.
