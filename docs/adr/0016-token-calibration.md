# ADR-0016: Calibrate the token estimate from real usage

Status: accepted (Phase 3)

## Context

Budgets use characters ÷ 4 for every model. That is wrong by a different amount per model and per kind of text, and memory
makes budgets matter more. Providers report real usage on every call, and the runtime already records the estimate and
the actual.

## Decision

- A `TokenCalibration` trait gives a **factor per model** and receives samples (estimated, actual). The default factor is
  1.0, so behaviour is unchanged until there is data.
- A sample compares the estimate (messages plus tool definitions) with the real prompt size
  (`input + cache_read + cache_write`). Samples from the simulated provider, with fewer than 512 estimated tokens, or (for
  Ollama) from continued steps are skipped.
- The ratio is clamped to 0.5–3.0 and folded in with an exponential moving average (α = 0.2). It is used after three samples.
- Stored per requested model reference and per serving `provider/model`; looked up by the reference.
- When the table is empty, past request/response event pairs seed it at half weight.
- Every budget and share is computed in calibrated tokens.

## Consequences

- No tokenizer dependency; works for models that publish none.
- Slow to learn and noisy at first; clamping bounds the damage and it only moves budgets.
- Tool definitions are now counted in the budget, not only in the router's estimate.
