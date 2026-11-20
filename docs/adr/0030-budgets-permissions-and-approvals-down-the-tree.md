# ADR-0030: Budgets and permissions only shrink down the tree, and approvals rise to the root

Status: accepted (Phase 6)

## Context

Delegation multiplies work: a lead that starts eight children, each of which may start more, can spend and reach far more than the person
meant. And a person who started one run should not have to open every descendant to answer an approval.

## Decision

**Permissions.** A child's grants are the **intersection** of the child agent's own grants (`agent_grants`) and the parent's, through
`AgentAsk.narrow` and `Grants::intersect`. A child can never do what its parent could not. An agent may ask another only with
`Execute` on `agent:<slug>` (access entry `agent:SLUG` or `agent:*`, or membership of a team). A task's `files` patterns narrow the
child's workspace reads further. The child's own model allow-list holds, and every candidate of its resolved route must also match the
parent's list, or the delegation is refused with the reason.

**Budgets.** At spawn, the parent's remaining cost, token and tool-call budgets are split equally among the call's children (at least 1
each); iterations and active time are the smaller of the child agent's limit and the parent's remaining. The children's usage is
**rolled up** into the parent's when they settle, so the next limit check at the parent sees it. Limits are checked as before.

**Guards.** Depth at most 3 (the root is 0), 8 tasks per call, 32 descendants per root, and no delegation to an agent already in the
ancestor chain. Children inherit the evaluation label and its refused approvals. A child's memory is narrowed by the parent's, and a
child has **no conversation**: its context is the handoff the lead wrote.

**Approvals.** A child asks for approval as any run does. The tree is addressable: `tree(root)` and `pending_in_tree(root)` list the
descendants and the approvals they wait on, with the execution that asked. The command line (`approve` and `deny` on a root id, and the
follow loop in `ask` mode) and the desktop (the root's run view and the lead's chat) decide an approval on the **child's id**, so the
existing guarantees hold: the decision is bound to the exact call by hash, only a person can approve `memory.remember`, and nothing
approves itself.

## Consequences

- The cost of a tree is bounded by the root's limits, whatever the fan-out.
- An equal split is coarse: a child that needs more than its share fails with a limit and says so in its result; adaptive splitting is
  future work.
- A descendant waiting for a person keeps the whole tree waiting, which is the honest state.
