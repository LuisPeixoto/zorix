# ADR-0003: A strict execution lifecycle with seven working states

Status: accepted (Phase 1)

## Context

The brief lists CREATED, QUEUED, INITIALIZING, RUNNING, WAITING_FOR_MODEL, WAITING_FOR_TOOL, WAITING_FOR_APPROVAL,
PAUSED, COMPLETED, FAILED, CANCELLED and asks which are really needed.

## Decision

Keep all but `INITIALIZING`. Building the context is a step inside `RUNNING` (it has no waiting and no external
visibility that a state would add). Retrying a model or tool call is an `attempt` counter on the step, not a state.

```
Created ──► Queued ──► Running ◄──► WaitingForModel
   │           │          │  ▲ ◄──► WaitingForTool
   │           │          │  └────► WaitingForApproval
   │           │          ├──────► Paused ──► Queued
   ▼           ▼          ▼
Cancelled   Cancelled   Completed | Failed | Cancelled      (terminal)
```

Transitions are one pure function `transition(state, trigger) -> Result<State, InvalidTransition>`. The repository
applies a transition only if the stored `version` equals the one the caller read (optimistic concurrency), so two
workers cannot move one execution. Terminal states accept nothing.

## Consequences

- An exhaustive test walks every `(state, trigger)` pair.
- `Paused` and `WaitingForApproval` survive a restart; `Running` and the other waiting states are re-queued by recovery.
