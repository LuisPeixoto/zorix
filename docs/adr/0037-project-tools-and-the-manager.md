# ADR-0037: Project tools and the manager

Status: accepted (Phase 8)

## Context

A project can have a **manager**: an agent that plans the work. Phase 9 adds virtual managers with autonomy levels, a weekly budget and
reports. Phase 8 needs only the part that is safe by the rules ZORIX already has: an agent can read and change tasks, and every change
asks a person first.

## Decision

- **Native tools** `tasks.list`, `tasks.get`, `tasks.create`, `tasks.update`, `tasks.move` and `tasks.comment`, registered with the other
  native tools. Reads do not ask. Every write asks, like `github.create_issue`.
- **`tasks.move` asks a person who is looking**: no automatic approval can give it, the rule `memory.remember` follows. A tool's
  approval is a property of the tool, not of one argument, so every move asks, a move to Done included. A task moving to Done under a
  manager is therefore an ordinary approval, with the exact change and a fingerprint, in the Inbox.
- **Grants are per project.** A run asked to manage a project gets `project:KEY`; the tools refuse any task of another project. This is
  the same shape as `github:owner/repo`.
- **The assignee's tools.** A task run gets `task:KEY`, which allows `task.report_progress`, `task.add_deliverable` and `task.note`
  on that one task. They change only the task's own record, so they do not ask. This is where real progress and deliverables come from.
- **Ask the manager.** *Ask Atlas* starts an ordinary run of the manager agent with the `project:KEY` grant and a prompt that names the
  project. While it is going its tasks say that the manager is planning.
- **You** move tasks freely, Done included. You are the person who approves.

## Consequences

- A manager cannot change a project without a person seeing it, and cannot touch another project.
- A task that an agent works on can report progress without being able to change anything else.
- Planning the week, autonomy levels and reports are Phase 9; people and sharing are Phase 16.
