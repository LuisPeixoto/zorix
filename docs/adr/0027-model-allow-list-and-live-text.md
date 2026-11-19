# ADR-0027: An agent's model allow-list lives in the router, and text streams live

Status: accepted (Phase 5)

## Context

An agent names the models it may use. A route can fall back from one provider to another, so a check made before the call is not
enough: the fallback must not escape the list. Separately, the conversation view needs text as it is written, and providers
already stream internally but return only the aggregate.

## Decision

- `ModelRequest.allowed_models` (patterns `provider/model` or `provider/*`, never aliases or routes, which can change).
  The runtime sets it from the execution's binding. `Router::plan` drops every candidate that matches no pattern, so
  fallbacks only iterate allowed candidates. When nothing remains the run fails with a clear reason. A gateway that bypasses the
  router is checked by the runtime before the call.
- **Live text**: `ModelRequest` carries an optional `TextSink`. The adapters' stream aggregators push text deltas into it; the
  router sends a reset before it tries a fallback. The runtime fans chunks out on an in-memory `LiveBus` that is **never
  persisted** and is not part of the event record. `ExecutionEngine::live` returns the stream (empty by default). When the
  stored `model.responded` event arrives, the stored text replaces the draft. Only text is streamed; reasoning deltas are not.

## Consequences

- The allow-list is enforced where fallbacks happen, with one small field on the request.
- Live text never changes what is recorded or replayed.
