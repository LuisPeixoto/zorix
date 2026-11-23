# ADR-0033: Workflows are versioned graphs that create executions

Status: accepted (Phase 7)

## Context

Agents and teams run when a person asks. Many jobs repeat or chain: every weekday at eight, when an issue is labelled, after another
job finished. They also stop for a person at known points. The runtime already gives us durable executions, limits that only shrink
(ADR-0030) and approvals bound to one call (ADR-0006). A workflow must reuse them, not replace them.

## Decision

- A **workflow** is a definition: triggers, steps and edges, an optional cost limit, and an optional "minutes saved per run". It is a
  **DAG** of at most 50 steps, validated on save: at least one trigger, unique step ids, no cycle, every step reachable from a trigger,
  and the two edges of a condition labelled `yes` and `no`.
- **Versions.** A workflow has one **draft** (what the editor edits) and immutable **published** versions v1, v2 … with a hash (the
  same canonical SHA-256 as agents) and a note. The next trigger uses the live version. Rolling back publishes an old definition as a
  new version. A draft that equals the live version is not a new version.
- A **workflow run** is its own record (`workflow_runs`): the version, what started it, the state of each step, usage and times. It
  does not hold a worker. Steps that need a model create **ordinary executions** through the hub (`prepare_agent_run`,
  `prepare_team_run`), with metadata `workflow_run` and `workflow_step`, so everything an agent run already has (tree, approvals,
  cost, events) applies, and Run view can link back to the workflow.
- **Steps.** Agent and Team; Condition, Loop (for each item, at most 8) and Wait; Approval and Ask a question; Send message (Slack or
  Discord) and HTTP request. Values pass between steps with templates (`{{trigger.payload.x}}`, `{{steps.<id>.output}}`); a missing
  path is an error, never an empty string. Conditions use a small expression language with paths, literals, comparisons, `and`,
  `or`, `not` and `contains`, evaluated over JSON, and nothing else.
- **Joins and skips.** A step becomes ready when every input edge is settled. The branch a condition does not take is marked
  *skipped*, and a skip passes downstream until a step has at least one input that ran. At most 8 steps are active at once.
- **Budget.** A workflow run may carry a cost limit. Each agent or team step gets the smaller of its own limits and what is left, and
  cost rolls up to the workflow run, as it does for a delegation tree.
- **Test run.** Runs the draft, marked as a test, and every action asks first whatever the step says.
- **File.** `<slug>.workflow.json` with `"zorix": "workflow/1"`; canonical, and strict about unknown keys. Embedded starters install
  as a draft that is off.

## Consequences

- Workflows are a client of the `ExecutionEngine` contract and add one crate, `zorix-workflows`; the runtime does not change.
- Teams stay unversioned; a workflow records the version of each agent it started, through the executions it creates.
- A fixed graph is less flexible than a model deciding the plan; that is the point, and `agents.delegate` stays for the open case.
- Left out for now: a pull request action (no tool creates branches or commits), polling events such as "new ticket", and people and
  rotations as approvers (Phase 16).
