# Persistence and recovery

State is saved after every step, so a stopped execution does not have to start again ([ADR-0004](adr/0004-sqlite-persistence.md)).

## Where the data lives

| Platform | Default folder |
|---|---|
| macOS | `~/Library/Application Support/zorix` |
| Windows | `%APPDATA%\zorix` |
| Linux | `~/.local/share/zorix` |

Set `ZORIX_HOME` (or `--home`) to use another folder. Tests, demos and screenshots always do, so they never touch real data.
`zorix data-dir` prints the folder in use.

| File | Holds |
|---|---|
| `zorix.sqlite` (+ `-wal`, `-shm`) | Executions, events, checkpoints, tool calls, approvals, leases |
| `notes.json` | The built-in notebook (demo tool) |
| `zorix.lock` | The lock that keeps two ZORIX programs from using the folder at once |

## The database

SQLite through the `ExecutionRepository` trait, in WAL mode so a reader (the CLI looking around) works while the engine
writes. The schema is numbered (`PRAGMA user_version`); a database written by a *newer* ZORIX is refused rather than
risked.

| Table | Holds |
|---|---|
| `executions` | The execution as JSON, plus the columns queried (tenant, state, version, idempotency key) |
| `events` | The event log, with a global `seq` |
| `checkpoints` | The latest resumable state of each execution |
| `tool_calls` | One record per call: arguments, hash, status, result, attempts, whether it may be repeated |
| `approvals` | Requests and decisions |
| `leases` | Which worker is working on which execution |

**One transaction per change.** `ExecutionRepository::commit` writes the execution, its events, the checkpoint, and the
tool call and approval records together. Each commit names the version it read; if the stored version differs, it fails
with a conflict and changes nothing. That is what prevents two workers (or the UI and a worker) from overwriting each
other.

## What a checkpoint holds

The conversation so far (assistant messages and tool results) and the tool calls from the last answer that are still
pending. To continue, the loop only needs to ask: *is a call pending? run it. Otherwise ask the model.*

## Recovery

`recover()` runs when a program starts (the CLI's `zorix recover` and the desktop app do it for you).

| Found | What happens |
|---|---|
| `queued` | Put in the queue again |
| `running`, `waiting_for_model` | Re-queued; continues from the last checkpoint. The model call is repeated (it has no side effect) |
| `waiting_for_tool`, call is **idempotent** | Re-queued; the tool runs again |
| `waiting_for_tool`, call is **not** idempotent | **A person decides.** It becomes `waiting_for_approval` with the reason *"…it is not known whether it finished"*. Approve to run it again; refuse to skip it |
| `waiting_for_approval`, `paused` | Left as they are |

A tool that finished before the crash is **not** run again: its result is in the checkpoint.

## Leases, duplicates and idempotency

- **Leases.** A worker takes a lease (with a time limit and a heartbeat) before working on an execution. Another worker
  or process cannot take it. `recover()` skips executions whose lease a live engine holds.
- **Sole owner.** The CLI and the desktop app lock the data folder, so they know no other process is using it. They run
  recovery with `sole_owner`, which drops leases a crashed process left behind and takes over at once. Without it,
  recovery waits for those leases to expire.
- **Create is idempotent.** Sending the same `idempotency_key` for the same tenant returns the same execution and stores
  nothing new (`zorix run … --idempotency-key invoice-42`).
- **Start is idempotent.** Starting an execution that already started changes nothing.
- **Approvals once.** An approval can be answered once.
- **Queues dedupe.** Asking for the same execution to run twice while it waits queues it once.

## Backups

Copy the data folder while ZORIX is closed. SQLite files are ordinary files.

## Limits

- One machine, one process at a time (enforced by the folder lock). A shared store for teams is a later backend behind
  the same trait.
- Secrets are never stored (there are none in this phase). Prompts, tool arguments and results **are** stored, as they are
  the record of what happened: treat the data folder as sensitive.
- Retention policies (deleting old executions automatically) are not implemented yet.
