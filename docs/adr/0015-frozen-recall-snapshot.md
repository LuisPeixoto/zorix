# ADR-0015: Memory is retrieved once per execution and frozen

Status: accepted (Phase 3)

## Context

The context is rebuilt on every model turn. Providers bind reasoning to the exact conversation prefix (ADR-0012) and cache
that prefix. If retrieved text changed between turns of one execution, the prefix would change, native turns would be
rejected or ignored and the cache would be lost. A crash and resume must also produce the same request.

A related bug: `fit()` marks the history as edited only when a *history* segment is dropped. Memory segments sit before the
history, so dropping one later in a run changes the prefix while native turns are still replayed.

## Decision

- The first `build` of an execution retrieves and stores a **recall snapshot**: the items (ids, text, citations, scores), the
  query, the embeddings model and a degraded reason. It is written with insert-or-ignore, so the first writer wins; later
  turns and resumes load it.
- Every segment is **rendered from the snapshot by a pure function**, so the bytes are identical on every turn.
- Explicit tool calls (`memory.search`, `knowledge.search`) are a separate, visible step and do not alter the snapshot.
- `fit()` sets `history_edited` when it drops **any** segment while the transcript holds native turns.
- Snapshots record which items they hold, so deleting an item scrubs the snapshot.
- Retrieved data is user-role, untrusted and wrapped; it never goes in the system prompt (adapters hoist system messages).

## Consequences

- Memory changes made during a run are not seen by that run (they are by the next).
- Prompt caching and preserved thinking keep working.
- Embedding the query costs a call per execution; its cost is recorded on the snapshot, not in the run's usage.
