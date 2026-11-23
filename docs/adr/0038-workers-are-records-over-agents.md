# ADR-0038: Workers are records over agents

Status: accepted (Phase 9)

## Context

Phase 9 puts agents in an organisation: a role, a department, someone to answer to, a level of autonomy and a budget. ZORIX already has
agents (versioned, hashed definitions, [ADR-0024](0024-agents-and-versions.md)) and teams (a lead and the colleagues it may
ask, [ADR-0031](0031-teams-and-the-handoff-format.md)). A team is a delegation group, not an organisation.

## Decision

- **A worker is a record over an existing agent**: `agent` (one worker per agent), `role`, `department`, `reports_to` (you, or another
  worker), `manager` (a virtual manager), `autonomy`, a weekly budget, a time limit per task, access patterns that always ask, and a
  schedule. The agent and its versions do not change, so hiring never alters a hash or a definition.
- **Departments** are a name, a description and an optional lead. They group workers for people to read. Teams stay what they were.
- **Every chain ends at you.** `reports_to` has no cycles. The only person is you until accounts exist (Phase 16), so you are the root of
  the chart and the one who approves.
- **Trees shrink.** A worker's autonomy is at most its manager's, and its weekly budget is at most its manager's. This is the rule a run
  tree already follows ([ADR-0030](0030-budgets-permissions-and-approvals-down-the-tree.md)), applied to the organisation.
- **Figures are derived, never stored.** Tasks come from projects, the on-time rate from the tasks finished in the last 30 days against their
  due date, the eval score from the latest finished evaluation, and spend from the stored cost of the worker's runs. A manager's figures
  include everyone who reports to it. A figure with nothing behind it reads as a dash.
- **Spend** is counted from the start of the week (Monday, in the local time zone), per worker, without the cost of the runs it
  delegated, so nothing is counted twice, and without evaluation runs.
- **Organisation settings** are one record: a weekly budget for everyone (optional), the level new workers start at, and the departments.

## Consequences

- An agent works the same with or without a worker record; removing the record changes nothing about the agent.
- The chart, the list and the profile show the same numbers, because they are computed in one place.
- Other people, a daily budget and teams as workers are left for later phases.
