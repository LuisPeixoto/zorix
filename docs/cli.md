# Command line reference

\`\`\`
zorix [--home DIR] [--tenant NAME] [--json] <command>
\`\`\`

Arguments are checked before anything touches the data folder: a bad command line exits with code 2 and creates nothing.

## Global options

| Option | Meaning |
|---|---|
| \`--home <DIR>\` | The data folder. Also read from \`ZORIX_HOME\`. Default: the platform's application data folder |
| \`--tenant <NAME>\` | Whose work to use (letters, digits, \`-\`, \`_\`, \`.\`, \`:\`; up to 64). Default \`local\`. Other tenants cannot see it |
| \`--json\` | Machine-readable output. \`run\`, \`approve\`, \`deny\` and \`resume\` print one JSON event per line on stdout and end with \`{"type":"result","execution":…}\` |

Progress and messages go to **stderr**; the answer goes to **stdout**, so \`zorix run … | next-command\` receives only the answer.

## Commands

### \`zorix run <PROMPT>\`

Starts an activity and follows it until it finishes, needs you, or you press Ctrl-C.

| Option | Meaning |
|---|---|
| \`--model demo\\|script:<file>\` | The model. \`demo\` is a rule-based simulator. A \`script\` is a JSON list of turns, for tests (its position lives in the process, so use it with \`--approve ask\\|yes\\|no\`) |
| \`--instructions <TEXT>\` | Standing instructions for the model |
| \`--allow <TOOL>\` | Let the run use a tool (an id, or a prefix ending in \`*\`). Repeat for more |
| \`--allow-all-tools\` | Let the run use every built-in tool. Tools that change things still ask first |
| \`--workspace <DIR>\` | The folder file tools are confined to |
| \`--allow-read <PATTERN>\`, \`--allow-write <PATTERN>\` | Paths file tools may read or write, relative to the workspace (\`*\`, \`docs/*\`) |
| \`--approve ask\\|never\\|yes\\|no\` | What to do when a call needs approval. \`ask\` prompts on the terminal; \`never\` leaves it waiting and exits; \`yes\` and \`no\` answer automatically (for demos and tests). Default: \`ask\` on a terminal, \`never\` otherwise |
| \`--max-iterations N\` (1–1000), \`--max-tool-calls N\`, \`--max-seconds N\` | Limits |
| \`--conversation ID\` | Group executions of one conversation |
| \`--idempotency-key KEY\` | Running again with the same key returns the same execution instead of making a new one |

Ctrl-C **pauses** at the next step (continue with \`zorix resume\`). A second Ctrl-C stops at once; continue later with \`zorix recover\`.

### \`zorix list [--state STATE] [--limit N]\`

Executions, newest first.

### \`zorix show <ID>\`

Model, steps, tool calls with timings, usage, pending approvals, failure or result. \`--json\` prints \`{summary, execution, approvals}\`.

### \`zorix events <ID> [--follow] [--after SEQ]\`

The recorded events. \`--follow\` keeps printing until the execution finishes, waits for approval or pauses. It only reads
the store, so it works while another ZORIX program is running.

### \`zorix approve <ID> [APPROVAL] [--note TEXT]\` and \`zorix deny …\`

Answer the pending approval (the id is optional when there is exactly one), then follow the execution. They show what is
being decided. \`--then ask|never|yes|no\` says what to do if the run asks for something else afterwards (default \`never\`).

### \`zorix cancel <ID> [--reason TEXT]\`

### \`zorix resume <ID>\`

Continue a paused execution and follow it.

### \`zorix recover [--no-wait]\`

Continue executions that a crash or a closed terminal left in the middle of a step. Executions whose last tool call may
not be repeated are put in \`waiting_for_approval\` with an explanation. See [persistence](persistence.md#recovery).

### \`zorix tools\`

Lists the built-in tools and how careful each is. \`zorix data-dir\` prints the data folder.

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

Commands that run work (\`run\`, \`approve\`, \`deny\`, \`resume\`, \`cancel\`, \`recover\`) lock the data folder. If another
ZORIX program (the desktop app, another terminal) holds it, they fail with *"Another ZORIX program is using the data
folder"*. Reading commands (\`list\`, \`show\`, \`events\`, \`tools\`) never lock.
