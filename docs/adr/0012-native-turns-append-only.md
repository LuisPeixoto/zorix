# ADR-0012: Keep the provider's raw turn and keep history append-only

Status: accepted (Phase 2)

## Context

Some providers bind reasoning state to the exact conversation: Anthropic's thinking blocks are tied to the prefix that
produced them (editing earlier turns invalidates them), Gemini requires `thoughtSignature` on function-call turns, and
OpenAI wants reasoning items passed back with tool outputs. A portable "role and text" message cannot carry that.

## Decision

An assistant message keeps, next to its portable form, the provider's **raw turn** (`Message.native`: provider, model,
content). An adapter replays it **unchanged** when the next call goes to the same provider and model, and sends the
portable form otherwise (so a run can continue on another model). The context builder is **append-only**: it never edits
earlier turns. When a small context window forces it to drop old turns it marks the request `history_edited`, and adapters
then send the portable form without native blocks, which every provider accepts.

## Consequences

- Cross-model continuation works; same-model continuation keeps the provider's reasoning state and its prompt cache.
- Checkpoints grow by the size of the raw turns. Old (0.1) checkpoints have no `native` and load unchanged.
- A provider that changes its binding rules affects only its adapter.
