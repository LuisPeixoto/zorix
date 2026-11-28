# Command line reference

```
zorix [--home DIR] [--tenant NAME] [--json] <command>
```

Arguments are checked before anything touches the data folder: a bad command line exits with code 2 and creates nothing.

## Global options

| Option | Meaning |
|---|---|
| `--home <DIR>` | The data folder. Also read from `ZORIX_HOME`. Default: the platform's application data folder |
| `--tenant <NAME>` | Whose work to use (letters, digits, `-`, `_`, `.`, `:`; up to 64). Default `local`. Other tenants cannot see it |
| `--json` | Machine-readable output. `run`, `approve`, `deny` and `resume` print one JSON event per line on stdout and end with `{"type":"result","execution":…}` |

Progress and messages go to **stderr**; the answer goes to **stdout**, so `zorix run … | next-command` receives only the answer.

## Commands

### `zorix run <PROMPT>`

Starts an activity and follows it until it finishes, needs you, or you press Ctrl-C.

| Option | Meaning |
|---|---|
| `--model <MODEL>` | `provider/model` (`anthropic/claude-opus-5-5`), `route:<name>`, an alias, `demo` or `script:<file>`. Default: the `default` alias, else Anthropic when it has a key, else `demo`. See [providers](providers.md#naming-a-model). A `script` is a JSON list of turns, for tests |
| `--local-only` | Use only providers on this computer. Nothing is sent to the cloud |
| `--instructions <TEXT>` | Standing instructions for the model |
| `--allow <TOOL>` | Let the run use a tool (an id, or a prefix ending in `*`). Repeat for more |
| `--allow-all-tools` | Let the run use every built-in tool. Tools that change things still ask first |
| `--allow 'mcp.<server>.*'` | Let the run use the tools of an MCP server (each tool still asks first; see [MCP](mcp.md)) |
| `--allow-http HOST`, `--allow-repo REPO[:write]`, `--allow-jira KEY[:write]`, `--allow-chat CHANNEL[:write]`, `--allow-db NAME[:write]`, `--allow-code LANGUAGE` | What the run may reach beyond its own tools: each gives the permission on the resource **and** the right to use the tools that reach it ([integrations](integrations.md)). Writes always ask |
| `--workspace <DIR>` | The folder file tools are confined to |
| `--allow-read <PATTERN>`, `--allow-write <PATTERN>` | Paths file tools may read or write, relative to the workspace (`*`, `docs/*`) |
| `--approve ask\|never\|yes\|no` | What to do when a call needs approval. `ask` prompts on the terminal; `never` leaves it waiting and exits; `yes` and `no` answer automatically (for demos and tests). Default: `ask` on a terminal, `never` otherwise. Only `ask` can answer what an MCP server asks while its tool runs ([MCP](mcp.md#what-a-server-may-ask-while-its-tool-runs)) |
| `--max-iterations N` (1–1000), `--max-tool-calls N`, `--max-seconds N` | Limits |
| `--conversation ID` | The conversation this run belongs to. Earlier runs of the same conversation are given to the model ([memory](memory.md)) |
| `--no-memory`, `--no-knowledge`, `--no-conversation` | Do not give this run facts and past work, indexed documents, or the earlier runs of the conversation |
| `--idempotency-key KEY` | Running again with the same key returns the same execution instead of making a new one |
| `--attach FILE` | Give the run a file to read: text, code, HTML or a PDF (a scan too, when [reading scans](knowledge.md#reading-scans) is on). Repeat for more, up to ten. The file is data, adds no permission, and is read before the run exists: a private name (`.env`, keys), a credentials folder, more than 1 MB of text (25 MB for a PDF) or a file that cannot be read stops it with the file named ([ADR-0049](adr/0049-files-attached-to-a-run.md)) |

Ctrl-C **pauses** at the next step (continue with `zorix resume`). A second Ctrl-C stops at once; continue later with `zorix recover`.

**As an agent.** `--agent NAME` runs the prompt as an [agent](agents.md): it brings its instructions, skills, tools, access and memory.
The other flags only **narrow** it: an `--allow…` flag keeps what both the agent and the flag allow, `--max-…` flags take the smaller limit,
`--no-memory` and friends switch memory off. `--agent-version N` picks a version, `--input JSON` sends data with the prompt, and `--model`
must be one the agent may use. `--instructions` cannot be combined with it. The version is printed (`zorix: calc v2`) and shown by `zorix show`.

**As a team.** `--team NAME` runs the team's lead and lets it ask the members (not together with `--agent`). While the run waits for the agents it asked, `--approve ask` answers
their approvals in the same terminal, and the progress of each sub-run is printed under its agent with `↳`. Without `ask`, a run whose sub-run needs a person exits with code 3 and says
`zorix approve ROOT`.

### `zorix list [--state STATE] [--agent NAME] [--all] [--limit N]`

Executions, newest first. Only **root** runs are listed; `--all` adds the sub-runs (marked `↳`).

### `zorix tree <ID>`

The tree the run belongs to, drawn from its top (any run in the tree will do): agent and version, state, steps, cost and the task. `--json` prints the executions.

### `zorix team …`

`team list`, `show NAME`, `create NAME --lead AGENT [--member AGENT]… [--name N] [--description D]`, `edit NAME [--lead A] [--add-member A]… [--remove-member A]…`, `remove NAME --yes`,
`add --template research-desk` and `templates` ([teams](teams.md)). `approve` and `deny` take the root of a tree and answer the approval that waits anywhere below it.

### `zorix show <ID> [--context]`

Model, steps, tool calls with timings, usage, pending approvals, failure or result. `--context` adds what memory put into the run (facts, earlier exchanges, document pieces with their sources, which search found each) and what each part of the first request cost in tokens. `--json` prints `{summary, execution, approvals, context}` (`context` is `null` without `--context`).

### `zorix events <ID> [--follow] [--after SEQ]`

The recorded events. `--follow` keeps printing until the execution finishes, waits for approval or pauses. It only reads
the store, so it works while another ZORIX program is running.

### `zorix approve <ID> [APPROVAL] [--note TEXT]` and `zorix deny …`

Answer the pending approval (the id is optional when there is exactly one), then follow the execution. They show what is
being decided. `--then ask|never|yes|no` says what to do if the run asks for something else afterwards (default `never`).

### `zorix cancel <ID> [--reason TEXT]`

### `zorix resume <ID>`

Continue a paused execution and follow it.

### `zorix recover [--no-wait]`

Continue executions that a crash or a closed terminal left in the middle of a step. Executions whose last tool call may
not be repeated are put in `waiting_for_approval` with an explanation. See [persistence](persistence.md#recovery).

### `zorix models [--available]`

Lists the models ZORIX knows, with context window, capabilities and price. `--available` first asks each provider that has
a key (or runs here) what it offers.

### `zorix provider list | set-key <NAME> [--stdin] | remove-key <NAME> | test <NAME>`

`list` shows each provider, cloud or local, and whether it has a key. `set-key` stores a key in the system keychain, read
from a hidden prompt (or one line of stdin with `--stdin`); it is never taken from an argument and never printed.
`test` asks the provider for its models: it checks the key and the connection and sends no conversation. See
[credentials](providers.md#credentials).

### `zorix route list`

Shows the aliases and routes from `providers.toml`.

### `zorix memory …`

`list [--scope global|project:<folder>|conversation:<id>] [--all]`, `add <TEXT> [--kind fact|preference] [--scope …] [--subject S] [--until YYYY-MM-DD]`,
`edit <ID> [--text T] [--subject S] [--until D | --no-until] [--scope …]`, `confirm <ID>`, `forget <ID> [--yes]`,
`search <QUERY> [--in memory|knowledge|history] [--project <folder>]` and `export [--out FILE]`. Ids look like `m:<id>`; the bare id works too.
Text that looks like a credential is refused. `forget` asks unless `--yes` is given, and refuses to run without a terminal. See [memory](memory.md).

### `zorix agent …`, `zorix skill …`, `zorix eval …`

Agents ([agents](agents.md)): `agent list [--all]`, `show NAME [--version N]`, `create NAME [--from-file F | --instructions T …]`, `edit NAME`
(opens `$VISUAL` or `$EDITOR`; a saved change is a new version), `import FILE [--as NAME] [--skills DIR] [--note T]`,
`export NAME [--version N] [-o FILE] [--with-skills DIR]`, `versions NAME`, `diff NAME A B`, `rollback NAME N`, `archive NAME [--undo]`,
`remove NAME --yes`, `add --template researcher|code-reviewer|data-analyst|writer [--as NAME]` and `templates`.
Skills ([skills](skills.md)): `skill list`, `show NAME`, `import <folder|zip>`, `remove NAME`, `gc`.
Evaluations ([evals](evals.md)): `eval add AGENT FILE.json`, `list AGENT`, `remove AGENT CASE`, `run AGENT [--version N] [--model M]… [--tag T]… [--offline] [--concurrency N]`
(exit code 1 when a case fails), `runs AGENT`, `show RUN` and `compare A B` (exit code 1 when something broke).
These take the data-folder lock only for `eval run`; the rest work while the desktop app is open.

### `zorix knowledge …`

`add <DIR> [--project <folder> | --global] [--include GLOB] [--exclude GLOB] [--auto-sync] [--no-sync]`, `list`, `sync [<FOLDER>]`,
`files <FOLDER> [--failed]`, `remove <FOLDER>`, `search <QUERY>` and `watch [<FOLDER>]` (keeps the folders marked with `add --watch` up to date until Ctrl-C). A folder is named by its path or the start of its id. Ctrl-C during a sync
stops it between files and exits with code 4; what was done stays indexed. See [knowledge](knowledge.md).

### `zorix run --estimate`

Say what the run would probably cost and how long it would take, without starting it: the median of your last similar runs (same agent and model), or a rough guess when there are fewer than three.
`--json` prints the numbers. With `--attach` it counts the files too (by their size; nothing is read). It needs no lock, so it works while the app is open. `--max-cost USD` limits what a run may spend (a model with no known price cannot keep it, and the run is refused).

### `zorix embeddings …`

`show`, `models`, `set <PROVIDER/MODEL> [--dims N] [--cloud]`, `off`, `test` and `reembed`. A cloud provider needs `--cloud`: your text is sent to it.

### `zorix settings …`

`show` and `set <KEY> <VALUE>`. The keys are `context.summaries` (on or off), `retention.days` (a number of days from 1 to 3650, or `off` to keep runs for ever), `retention.mode` (`compact` or `delete`), `telemetry.endpoint` (an `https://` address, or `http://` for a collector on this computer, or `off`), `telemetry.header` (`Name: value`, kept in the system keychain, or `off`) and `knowledge.ocr` (on or off; turning it on downloads the two OCR models once and checks them before the setting is saved, see [reading scans](knowledge.md#reading-scans)). A value that makes no sense is refused with a sentence and changes nothing. `--json` on `show` prints the whole record.

### `zorix retention …`

`show` says what a pass would do with the current settings; `run [--dry-run]` does it now (without `--dry-run` it holds the data-folder lock). Retention is off until `retention.days` is set. See [persistence](persistence.md#retention).

### `zorix workflow …`, `zorix serve`

Workflows: list, show, create, edit, import, export, versions, diff, publish, rollback, enable, disable, remove, run, runs, run-show, cancel, waits, answer, webhook, templates and add.
`serve` runs schedules, events and (with `--webhooks` or `--bind`) webhooks until Ctrl-C. See [workflows](workflows.md#the-command-line).

### `zorix project …`, `zorix task …`

Projects: list, create, show, edit, archive, restore, remove and ask. Tasks: list, add, show, edit, move, restart, comment, link, deliver, remove and history. `task move` to *In progress*,
`task restart` and `project ask` start a run and follow it (exit 3 when it waits for a person); the rest needs no engine. See [projects](projects.md#the-command-line).

### `zorix org …`, `zorix worker …`

The workforce: `org show`, `settings`, `dept add|edit|remove`, `activity` and `notices`; `worker list`, `hire`, `show`, `edit`, `level`, `fire`, `schedule`, `plan` and `report`. `worker plan` and `report` start a
run and follow it (exit 3 when it waits for a person); the rest needs no engine. See [workforce](workforce.md#the-command-line).

### `zorix mcp …`

Connect ZORIX to [MCP servers](mcp.md), or serve ZORIX over MCP.

| Command | What it does |
|---|---|
| `mcp catalog [SEARCH]` | The reviewed list of servers that can be added |
| `mcp add <NAME> (--catalog ID [--input NAME=VALUE] \| --command PROGRAM [--arg ARG] [--env NAME] [--set NAME=VALUE] [--cwd DIR] \| --url URL [--auth none\|bearer\|oauth] [--client-id ID] [--scope S])` | Add a server, **off**. `--env NAME` reads a secret from this command's environment variable of the same name (or asks); it goes to the keychain |
| `mcp list`, `mcp show <NAME>` | The servers, and one in full (secrets are never shown) |
| `mcp enable <NAME> [--yes]` | Turn it on. Shows what it will run and asks, unless `--yes`; without a terminal `--yes` is required |
| `mcp disable <NAME>`, `mcp remove <NAME>` | Turn off, or delete with every secret kept for it |
| `mcp tools <NAME>` | Start the server briefly and list its tools: *new*, *pinned*, *changed*, *disabled* |
| `mcp review <NAME> [--approve TOOL] [--approve-all] [--disable TOOL]` | Without flags, show what is new or changed (a changed description shows was and now) |
| `mcp trust <NAME> --tool TOOL (--read-only-without-asking \| --ask)` | Whether calls ask first; the first only for a tool the server reports as read-only |
| `mcp login <NAME>`, `mcp logout <NAME>` | Browser sign-in (OAuth) and forgetting it |
| `mcp import [--from claude\|vscode\|cursor] [--path FILE] [--yes]` | Shows what another program has configured; adds it (off) only with `--yes` |
| `mcp logs <NAME>` | Start briefly and show what the server wrote on its error stream |
| `mcp sandbox <name> [--mode native\|off] [--network on\|off] [--folder DIR] [--folder-write DIR] [--clear-folders]` | Show or choose how a local server is sandboxed ([MCP](mcp.md#the-sandbox-of-a-local-server)) |
| `mcp doctor` | What a sandboxed server is kept from on this computer |
| `mcp serve [--allow-runs] [--allow-approvals] [--allow-read-runs] [--project DIR] [--model M] [--allow TOOL] [--allow-read P] [--allow-write P]` | Serve ZORIX over MCP on standard input and output. Read-only unless `--allow-runs` |

`mcp serve` and the commands that only read (`mcp list`, `show`, `catalog`) take no lock.

### `zorix connect …`

`connect add github|gitlab|jira|slack|discord|postgres|sqlite|http|webhook <NAME> …`, `connect list`, `connect test <NAME>`,
`connect remove <NAME>`. See [integrations](integrations.md) for each kind's options. Secrets come from `--secret-env VAR`,
`--secret-stdin` or a hidden prompt, never from an argument. A webhook without a key gets one, shown once.

### `zorix sandbox status | install python`

What the [code sandbox](sandbox.md) can run, and downloading CPython for it (about 26 MB, checked against a fixed checksum).

### `zorix calibration`

Shows the factor learned for each model (how many real tokens an estimated token is worth) and how many samples it rests on.

### `zorix tools`

Lists the tools ZORIX has (built in, memory, the native integrations and the sandbox), where each comes from and how careful it is. MCP tools appear in the app and in a run once a server is running. `zorix data-dir` prints the data folder.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Completed |
| 1 | Failed, or the command could not be carried out (the reason is printed) |
| 2 | The command line was not valid |
| 3 | Waiting for a person's approval |
| 4 | Cancelled |
| 5 | Paused |
| 130 | Stopped by a second Ctrl-C |

## One program at a time

Commands that run work (`run`, `approve`, `deny`, `resume`, `cancel`, `recover`) lock the data folder. If another
ZORIX program (the desktop app, another terminal) holds it, they fail with *"Another ZORIX program is using the data
folder"*. Commands that change integrations (`mcp add`, `enable`, `review`, `tools`, `login`, `connect add`, `sandbox install`…) lock it too. Reading commands (`list`, `show`, `events`, `tools`, `mcp list`, `mcp catalog`, `connect list`) and `mcp serve` without `--allow-runs` never lock.
