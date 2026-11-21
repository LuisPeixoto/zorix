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
| `notes.json` | The built-in notebook (demo tool). Superseded by memory ([memory](memory.md)), kept for compatibility |
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

### Memory tables (schema 2)

| Table | Holds |
|---|---|
| `memory_items` (+ `memory_fts`) | Facts and preferences: scope, kind, subject, content, who proposed and who approved, dates, expiry |
| `knowledge_roots`, `knowledge_documents`, `knowledge_chunks` (+ `chunks_fts`) | Indexed folders, their files with a hash and a status, and the pieces |
| `operations` (+ `operations_fts`) | The record of every finished run |
| `vectors` | One L2-normalised vector per (model, text hash), so unchanged text is never embedded twice |
| `context_snapshots`, `snapshot_items` | What each run was given, frozen ([ADR-0015](adr/0015-frozen-recall-snapshot.md)) |
| `token_calibration`, `memory_settings` | The learned token factor per model; the embeddings model |

### Integration tables (schema 3)

| Table | Holds |
|---|---|
| `mcp_servers` | The servers a person added: name, transport (command, arguments, variable **names**, folder; or address and sign-in kind), whether it is on, the protocol era found |
| `mcp_tool_pins` | For each tool of a server: what it listed last, the definition and hash a person approved, its status (new, pinned, changed, disabled), whether it asks, who approved it and when |
| `connections` | Name, kind, settings that are not secret, whether a secret is kept, the last test |

No table holds a secret. They live in the system keychain under names such as `mcp.<id>.env.<variable>`, `mcp.<id>.token`,
`mcp.<id>.oauth` and `mcp.<id>.access` (OAuth), and `conn.<id>`; removing a server or connection removes its entries.
Opening a 0.3 database upgrades it in one transaction.

### Agent tables (schema 4)

| Table | Holds |
|---|---|
| `agents` | One row per agent: its id, slug, newest version, whether it is archived |
| `agent_versions` | Every version, never edited: the definition (JSON), its SHA-256, a note, the time |
| `skills`, `skill_versions` | What was imported: name, description, the files with sizes and hashes, license, source, and which version is current. The files themselves are in `<data>/skills/<sha256>/` |
| `eval_cases` | An agent's cases |
| `eval_runs` | One row per evaluation run, with its results |

`executions` gains an `agent` column (the agent, version and hash a run was made by) with an index, so the run list can be filtered by agent. Opening a
0.4 database upgrades it in one transaction; a 0.4 program refuses a schema-4 database instead of guessing. The agent scope of memory needs no table:
it is a scope value in the existing memory tables. Live text (the words of an answer while it is written) is never stored.

### Orchestration (schema 5)

| Change | Holds |
|---|---|
| `executions.parent`, `executions.root` | The run that asked and the run at the top of the tree, with indexes, so children, whole trees and root-only lists are queries. Filled for existing rows from the stored document |
| `teams` | One row per team: slug, name, description, avatar, lead, members and the time. A plain record, not versioned |

A call to `agents.delegate` is stored with the status `delegated` and, once the children finish, their results. Opening a 0.5 database upgrades it in one transaction; a 0.5
program refuses a schema-5 database instead of guessing.

**Recovery** also looks at `waiting_for_children`: a parent whose children are all finished is settled and queued; otherwise it is left waiting and each child recovers by itself.
A call is delegated at most once (the children's idempotency keys), so a restart never doubles the work.

### Projects (schema 7)

| Table | What it holds |
|---|---|
| `projects` | One row per project: key, whether it is archived, the counter that numbers its tasks (never goes down, so keys are not reused) and the record (`doc`) |
| `tasks` | One row per task: key, lane, place in the lane, a `revision` for optimistic writes and the record (its runs, deliverables, links and dependencies) |
| `project_events` | The history of a project, numbered from 1, with the task it concerns |

Writes begin immediate transactions, so two programs writing at once wait for each other instead of failing. Opening a schema-6 database upgrades it in one transaction; a 0.7 program refuses a
schema-7 database. Deleting a project removes its tasks and history.

### Workflows (schema 6)

| Table | Holds |
|---|---|
| `workflows` | One row per workflow: slug, whether it is on, the newest published version and the draft (`doc`) |
| `workflow_versions` | Published versions: number, hash, note, and the definition with the confirmations given for unattended actions. Never changed after they are written |
| `workflow_runs` | One row per run: state, start time, a `revision` for optimistic writes, and the whole record (the frozen definition, what started it, every step) |
| `workflow_waits` | The people and timers a run is stopped for: status, due time, and what the person was shown with its fingerprint |
| `workflow_triggers` | When each schedule fires next, when it last fired, and the fires missed while nothing was running |
| `workflow_meta` | Small values the host keeps, such as how far it has read the events |

Opening a schema-5 database upgrades it in one transaction; a 0.6 program refuses a schema-6 database instead of guessing. Deleting a workflow removes its versions and
trigger state and keeps its runs and waits, which carry their own copy of the definition.

Each search index is its own FTS5 table, kept in step by triggers. Opening a 0.2 database upgrades it in one transaction. Deleting a
memory or a folder removes its rows, search entries and vectors, scrubs the snapshots and overwrites the freed pages
(`secure_delete`).

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
| `waiting_for_children` | If every child is finished, the results are written and the parent is queued. Otherwise left waiting |

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
