# ADR-0031: Teams are plain records; the handoff is typed

Status: accepted (Phase 6)

## Context

Letting any agent call any agent is too open for most uses, and writing a per-agent list by hand is tedious. People think in teams: a
lead and the colleagues it may ask. The lead also needs to give a child a task that cannot be misread, and to get an answer it can use.

## Decision

- A **team** is a record: `slug`, `name`, `description`, `avatar`, a `lead` (an agent slug) and `members` (agent slugs), with the time it
  was changed. It is **not versioned**; each run still records the version of every agent in it. It is edited in the app or with
  `zorix team …`.
- **Running a team** runs the lead with extra grants `Execute agent:<member>` for each member and `use_tool agents.delegate`. The
  binding records the team's slug. Members are asked by the lead, never by each other unless their own access says so.
- `prepare` adds an **`<available_agents>`** index (slug, name, description) of everyone the run may ask, from `agent:` access or from
  the team, and says the limits on fan-out and depth, so the model can scale effort itself.
- The **handoff** is typed. A task is `{agent, task, input?, expected_output?, context?, files?}`: an objective, data, the shape of the
  answer as a JSON Schema, notes the lead chose to pass on, and workspace patterns. The child receives the task as its prompt and the
  rest as structured input. When `expected_output` is given, the child's answer is validated against it, and the parent receives both
  the text and the structured value, or the validation error.
- The starter team **`research-desk`** (a lead, `research-lead`, with a delegation skill and evaluation cases, plus the `researcher` and
  `writer` agents) installs with `zorix team add --template research-desk`.

## Consequences

- A team is a way to grant, not a workflow: fixed orchestration (sequences, conditions, schedules) is Phase 7.
- Unversioned teams can change under a running tree; the run's own record (binding and children) keeps what actually happened.
