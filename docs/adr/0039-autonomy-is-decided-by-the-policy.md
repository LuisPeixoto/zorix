# ADR-0039: Autonomy is decided by the policy

Status: accepted (Phase 9)

## Context

A worker has a level of autonomy: Suggest, Ask, Notify, Budgeted or Full. The design says the dial never overrides policy: a tool that
always asks still asks. Today one policy decides every call ([ADR-0006](0006-permissions-approvals.md)) and a call that the default policy
holds for approval waits for a person. The level has to change that answer for the runs the workforce starts, and nothing else.

## Decision

- **The level is frozen into the run** as `autonomy` (a level and the access patterns that always ask) on the agent binding, set only on
  runs the workforce starts: a task's run, a manager's run and a scheduled one. A run a person starts from *New run* is unchanged.
  A delegated child runs at the level of the run that asked it, with the same list of patterns that always ask, so work handed to another
  worker is never freer than the work that handed it over.
- **The policy applies it.** The rules stay in this order: forbidden, deny rules, grants, approval rules, then the level.
  - **Suggest**: a call that is not read-only does not run. The model is told it was proposed for a person to do, the call is recorded as
    a suggestion, and it appears in Activity and the Inbox.
  - **Ask**: every call that is not read-only asks, which is stricter than the default.
  - **Notify, Budgeted and Full**: a call that is held *only* because of the default (a destructive tool, or a tool that asks without being
    person-only) runs without asking. It is recorded with the tool, the exact arguments and the level.
  - **Still asks, at every level**: a tool that must be approved by a person who has seen it (`tasks.move`, `memory.remember`), a call that
    an explicit approval rule holds, and a call that needs an access pattern in the worker's own list.
- **Budgets are narrowings** ([ADR-0030](0030-budgets-permissions-and-approvals-down-the-tree.md)). Every run the workforce starts gets the time limit of its worker. At
  **Budgeted** its cost limit is the smaller of the agent's and what is left of the worker's week; at **Full** it is capped only by what
  is left of the organisation's. When the week is spent, a new run starts at **Ask** and says why.
- **Reporting follows the level.** Notify tells you at each unattended call; Budgeted lists them in the worker's week and tells you when
  the budget is near or spent; Full reports in the weekly report.
- **A manager's week.** *Plan the week* and *Write the report* run on demand. A worker that is a manager has a schedule (plan on Mondays,
  report on Fridays) that is off until it is turned on and that runs only while the app or `zorix serve` is open; a missed slot is recorded,
  not caught up.

## Consequences

- Loosening approvals is limited to runs the workforce starts, is off by default (new workers start at Ask), and is visible: each
  unattended call is an event with its arguments.
- Person-only tools refuse an unattended answer twice: in the policy and when an approval is decided.
- At the default policy a lot of tools still ask, so Notify, Budgeted and Full differ mostly in what they do not ask, in the reports they
  produce and in the money they may spend.
