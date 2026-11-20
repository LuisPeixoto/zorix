# ADR-0029: Delegation suspends the parent in a new state

Status: accepted (Phase 6)

## Context

An agent should be able to ask other agents to do part of the work, several at once. A tool that blocks until its children finish would
hold one of the engine's few workers for the whole time; with nested delegation, a handful of waiting parents would starve the pool and
nothing could make progress. The runtime already has a pattern for a run that must wait for something outside it: approvals.

## Decision

- Delegation is the tool **`agents.delegate`** `{tasks: [{agent, task, input?, expected_output?, context?, files?}]}` with 1 to 8 tasks.
  It goes through the normal checks (pins, argument validation, policy, approval) like any tool, and is then handled by the runtime
  itself, because it must change the parent's state.
- The runtime asks a **`DelegationPlanner`** (a port in `zorix-core`, implemented in `zorix-agents`) to turn each task into a
  `NewExecution`, creates the children with the idempotency key `delegate:{parent}:{call}:{index}`, marks the call **`Delegated`** with
  the children's ids, commits the new state **`WaitingForChildren`** with a checkpoint, starts the children and releases the worker.
- When a child reaches a terminal state, the engine looks at its parent. When every child of the pending call is terminal, the engine
  writes the **results** into the call's record, rolls the children's usage into the parent's, and re-queues the parent (trigger
  `ChildrenSettled`). On resume the pending call finds its result and the loop continues, as it does after an approval.
- A failed or cancelled child is **reported in the result**, not raised: the parent's model decides what to do.
- **`recover`** includes `WaitingForChildren`: if every child is terminal it settles and re-queues the parent; otherwise the parent keeps
  waiting and each child recovers on its own. Idempotency keys make a repeated create after a crash return the existing child.
- **Cancel cascades**: cancelling a run cancels its non-terminal descendants, leaves first. A child cancelled alone is only a result.
- `settle` treats `WaitingForChildren` as still working, except when a descendant is waiting for a person: then the tree is waiting.
- Events: `execution.waiting_children`, `execution.child.started` and `execution.child.settled`. The call's result is the typed object
  `{agent, version, task, state, answer, structured?, error?, steps, tool_calls, cost_micros, child_id}` per task.
- `executions` gains `parent`, `root` and `depth` as columns with indexes, so children and trees are queries, not scans.

## Consequences

- No worker is held while a parent waits; N waiting parents never deadlock a pool of N.
- A state is added to the lifecycle: every exhaustive match over `ExecutionState` is touched, and the transition table and its
  property tests are extended (ADR-0003 is amended, not replaced).
- A parent that crashes between creating a child and committing is safe because of the idempotency key; a child that finishes while the
  parent is committing its suspension is caught by the optimistic version retry and by `recover`.
- Delegation needs an agent or a team, because the list of whom to ask comes from preparing the run.
