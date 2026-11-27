# ADR-0044: Retention of finished runs

Status: accepted (Phase 11)

## Context

Nothing was ever removed. Every event, every checkpoint with the whole transcript, every tool call and every approval of every run stayed
for ever, and the roadmap asked for retention policies. Three things made it delicate: the cost of a run is read from the runs themselves, so
removing a run changes the spend figures and the workforce's budgets; a run can be part of a tree, and can be referred to by a task, a workflow
run or a report; and what a run stored may be exactly what a person needs to see why something happened.

## Decision

- **Off by default.** A person sets a number of days (1 to 3650); without one nothing is touched. The setting lives in the settings record
  ([ADR-0043](0043-summaries-of-long-conversations.md) uses the same one).
- **Two modes.** *Compact* (the default) removes what is bulky: events, checkpoints, tool call records, approvals, the recall snapshot and its
  items, and any lease. The run row stays with its request, its answer, its failure, its usage, its cost and its times, flagged `compacted`, so lists,
  spend figures and budgets do not change and the run still shows what it asked and answered. *Delete* removes the run and the past work recorded from
  it (and scrubs that text from the frozen recall of other runs), except a run that a task, a workflow run, a report or an organisation event refers to:
  that one is compacted instead, and the report says so.
- **Only what is finished, and only whole trees.** A run is eligible when its state is final and it finished before the cutoff, and so is every run in
  its tree. A tree with something still moving, or something newer, is left alone, so no parent loses its children's story.
- **Small transactions, and a preview.** Runs are handled 200 at a time, each batch in its own immediate transaction, so a pass never holds the
  database for long; `secure_delete` is on, and the log is checkpointed at the end so the removed text does not stay in a file. `preview` counts the
  same rows without changing anything, and the interface and the command line show it before anything is done.
- **When it runs.** Ten minutes after a program that keeps running starts (the desktop app, `zorix serve`) and then once a day, reading the settings
  each time; and on demand: `zorix retention show|run [--dry-run]` and the *Compact now* / *Delete now* button, which asks first.

## Consequences

- A compacted run cannot be replayed, resumed or inspected step by step. That is the point, and the screens say it.
- *Delete* does lose the spend of the runs it removes from any list that sums over runs. Budgets are weekly, so a run older than the retention is
  past every budget that counted it.
- Past work recorded from a compacted run stays: it is memory, with its own controls ([memory](../memory.md)).
- Retention knows only the references that are in the database. A person who exports a record of a run elsewhere keeps it there.
