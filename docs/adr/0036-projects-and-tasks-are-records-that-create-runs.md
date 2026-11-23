# ADR-0036: Projects and tasks are records that create runs

Status: accepted (Phase 8)

## Context

Phase 8 adds projects and tasks: a key (`OPS-38`), a priority, an assignee that is a person or an agent, progress, dates, dependencies,
links, deliverables and a history. A task given to an agent should make that agent work. ZORIX already has one way to run a model, the
execution, and workflows ([ADR-0033](0033-workflows-are-versioned-graphs-that-create-executions.md)) set the pattern: a record that
creates ordinary executions and follows them. There are no accounts yet (Phase 16), so the only person is the one using ZORIX.

## Decision

- **Records.** A *project* has a key (2 to 5 capitals, unique), a name, a description, an optional manager (an agent), optional start and
  due dates, an optional cost limit for each task run, and an archived flag. A *task* belongs to one project, has the key `KEY-n` from a
  per-project counter that is never reused, a title, a description, a state, a priority, an assignee, progress from 0 to 100, dates,
  dependencies, links, deliverables, its current and past runs, and its place in the lane. Both live in SQLite (schema 7) as a JSON `doc`
  with the same revision rule as workflow runs. Every change also writes one `project_events` row, which is the history.
- **Fixed lanes.** Backlog, In progress, Review and Done. Custom lanes are left out: the rules below depend on the meaning of each lane.
- **People.** An assignee is *you*, an agent, a team, or nobody. The history says "you". Accounts and sharing wait for Phase 16.
- **A task makes a run.** Moving a task with an agent or team to **In progress** starts an ordinary execution with the metadata
  `project` and `task`, an idempotency key, and limits that are the smaller of the agent's and the project's, as in ADR-0030. The run is
  the work; the task is the record around it.
  - A run that **finishes** moves the task to **Review** and its final answer becomes a deliverable.
  - A run that **fails** leaves the task in In progress, marked failed.
  - Moving the task out of In progress while its run is going **cancels** the run, after a confirmation.
  - From Review, **Send back** with a note starts the run again with that note in its prompt.
- **Dependencies.** A task may depend on tasks of its own project, with no cycles. A task with open dependencies can be moved to In
  progress, but its run waits and the card says it is blocked; the run starts when they are Done.
- **Needs you is derived.** It is not stored. A task needs you when it is in Review, when its run failed, or when its run waits for a
  person. The same facts put an item in the Inbox.
- **Reconciling.** One function, `reconcile`, reads the runs of tasks and starts, finishes or unblocks them. It runs on every read and on
  the host's beat ([ADR-0034](0034-the-workflow-host-and-triggers.md)), so the command line works with no host and an open app or
  `zorix serve` keeps tasks moving. It is idempotent.
- **Deliverables** are the final answer of a run, items an agent adds with its own task tools, and links or files a person attaches.
- **Links** are URLs. A pull request on GitHub or GitLab shows its number, and its state when a matching connection can read it.

## Consequences

- No second way to run a model, and cost, approvals, events and the work tree of a task run are the ones a run already has.
- Tasks do not need a host to be correct, only to move on their own.
- Not covered: custom lanes, recurring tasks, a workflow step or trigger for tasks, and import or export of a project.
