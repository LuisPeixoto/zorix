# ADR-0007: A context builder made of ordered, budgeted sources

Status: accepted (Phase 1)

## Context

The model input mixes system instructions, the task, history, tool results, project facts and, later, memory and
retrieved knowledge. Phase 3 will add memory; Phase 1 must not block it.

## Decision

`ContextBuilder::build(&ExecutionView) -> ModelRequest` is a trait. The default implementation runs ordered
`ContextSource`s (system, task, execution info, history, tool results, extra data). Each source returns segments with a
priority and a size estimate. When the estimate exceeds the budget, lowest-priority, oldest segments are dropped first
and a note records the omission; system instructions and the task are never dropped. Memory arrives later as another
`ContextSource` with no change to the engine.

## Consequences

- Token counting is an estimator (characters / 4) in Phase 1, replaceable per model in Phase 2.
