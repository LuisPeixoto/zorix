# Troubleshooting

## "Another ZORIX program is using the data folder"

The command line and the desktop app lock the data folder while they work, so only one of them runs work at a time.
Close the other one (or run `zorix list`, `show` and `events`, which never lock). If a program crashed, the lock is
released by the operating system; just try again.

## An execution is stuck in `running`, `waiting_for_model` or `waiting_for_tool`

The program that was running it stopped (a crash, a closed terminal, a second Ctrl-C). Run `zorix recover`, or start the
desktop app, which recovers on start-up. It continues from the last checkpoint.

## `zorix recover` says an execution is waiting for a decision

A tool that cannot safely be repeated was running when the program stopped. ZORIX does not know whether it finished, so a
person decides: `zorix approve <id>` runs it again, `zorix deny <id>` skips it. The reason shown starts *"ZORIX stopped
while … was running"*.

## "This execution has no permission for: use_tool …"

Nothing is allowed by default. Add `--allow <tool>` (or tick the tool in the desktop app). For file tools also pass
`--workspace <dir>` and `--allow-read` / `--allow-write` patterns.

## "No workspace folder is configured for this execution"

File tools need `--workspace <dir>` (or a workspace in the desktop form).

## "The path may not go up with “..”" and similar

The file tools only accept relative paths with forward slashes, inside the workspace. Absolute paths, `..`, drive letters,
backslashes and Windows reserved names are refused on every operating system, by design.

## `zorix run` exits with code 3

The execution is waiting for your approval. That is not an error. Answer with `zorix approve <id>` or `zorix deny <id>`,
or run with `--approve ask` to be asked on the terminal.

## "This database was made by a newer version of ZORIX"

You opened a data folder with an older program. Update ZORIX; the database is left untouched.

## The notes file is damaged

`notes.json` is only used by the demo notes tools. ZORIX refuses to overwrite a damaged file. Fix or delete it.

## Linux: the desktop app does not build

Install the Tauri system packages (see [getting started](getting-started.md#requirements)).

## Windows: SmartScreen or macOS: "cannot be opened"

The first releases are **not signed or notarized**. See [distribution](distribution.md).

## I want to see exactly what happened

`zorix events <id>` prints every event in order; `zorix show <id> --json` has the summary and timings. Set
`ZORIX_LOG=debug` (command line and desktop app) for logs on stderr; prompts, arguments and results are never written to logs.

## "There is no default model" or runs use the demo model

No provider has a key and no `default` alias is set. Store a key (`zorix provider set-key anthropic`), set
`default = "ollama/<model>"` under `[alias]` in `providers.toml`, or pass `--model`.

## `authentication` or "no key" for a provider

`zorix provider list` shows which providers have a key. Store one with `zorix provider set-key <name>`; an environment
variable such as `ANTHROPIC_API_KEY` is used first. On a server without a keychain set `ZORIX_SECRETS=env`.

## "Is Ollama running?"

Start it with `ollama serve` and fetch a model that supports tools, for example `ollama pull qwen3:1.7b`. `zorix provider test ollama`
checks the connection. A model without tool support is skipped by routes that need tools.

## Keychain unavailable

On Linux the keychain needs a running Secret Service (GNOME Keyring or KWallet). Without one, use an environment variable
and `ZORIX_SECRETS=env`.

## A run switched models

That is a fallback: the first candidate was unavailable, rate limited, refused or lacked the key. The `execution.model.fallback`
event says why; `zorix show <id>` lists the models that served the run. A model named exactly (`provider/model`) never falls back.

## A run did not use something I told it to remember

Look at `zorix show <id> --context` (or **Context** in the run view). Memory is chosen once, when the run starts: a fact added
afterwards is not in a run that was already going. Check that the run was not started with `--no-memory`, that the fact's scope
reaches the run (a *project* fact needs a run whose workspace is that folder), and that it has not expired or been replaced.

## "Waiting for a person" after `--approve yes` on a memory proposal

Intended: `memory.remember` can only be approved by a person. Approve it from the desktop or with `zorix approve <id>`.

## Documents are not found, or a file is "left out"

`zorix knowledge files <folder> --failed` says why for each file: ignored by `.gitignore`, too big, binary, a scanned PDF with
no text, or private (`.env`, keys). Add patterns with `--include`. Sync again after editing files; there is no file watcher.

## Search by meaning does nothing, or "search was weaker than usual"

`zorix embeddings show` names the model and how many texts have vectors; `zorix embeddings test` tries it. With Ollama run
`ollama pull all-minilm` first. After choosing a model run `zorix embeddings reembed`. When the model cannot be reached a run
continues with keyword search and its context says so.

## `zorix memory` or `zorix knowledge sync` says the data folder is in use

Writing commands need the data folder to themselves, so they fail while the desktop app is open. Close it, or use the app.

## An MCP server is "Starting" for a long time, or "Failed"

Open the server's **Logs** tab, or run `zorix mcp logs <name>`. Servers started with `npx` need Node installed and may download
their package the first time (the start waits up to 30 seconds). Remote servers need to be reachable over HTTPS.

## A server says "Sign in"

`zorix mcp login <name>` (or the **Sign in** button) opens your browser. If the browser cannot open, the address to open is
printed. Servers that need a client registered in advance take `--client-id`. If a sign-in keeps failing, `zorix mcp logout
<name>` and try again.

## A tool I approved is gone from runs

Its description or schema changed on the server, so it is withdrawn until you review it (`zorix mcp review <name>`, or the
server's drawer). This is deliberate: a changed description is a changed tool.

## A run says "There is no tool called mcp.…"

The run was not granted the server (`--allow 'mcp.<server>.*'`), the server was not running or not approved when the run
started, or the tool is off. Runs pin their tools when they start, so approving afterwards helps the next run.

## `http.get` says a host is not allowed, or an address is refused

A run reaches only hosts it was granted (`--allow-http <host>`). Addresses on this computer or a private network (`localhost`,
`10.…`, `192.168.…`, `169.254.…`) are refused unless that exact host is granted.

## Python says it is not installed

`zorix sandbox install python` (or **Install Python** under Integrations › Connections). JavaScript needs nothing.

## A connection test fails

The message says what the service answered, without secrets. Check the token's scopes (a GitHub token needs repository
access to write), the site address (Jira Cloud: `https://<name>.atlassian.net`), and for Postgres that the server offers TLS
(`sslmode disable` is only accepted for a database on this computer).

## "There is no agent called …"

`zorix agent list --all` shows every agent, archived ones included; an archived agent still runs by name. Names are lowercase letters, digits and dashes.

## "The skill … is not in the library"

An agent file pins skills by hash, and the files are not here. Import the folder (`zorix skill import FOLDER`), or give the folder that came with the file:
`zorix agent import FILE --skills DIR` (what `zorix agent export … --with-skills DIR` wrote). `zorix skill gc` deletes the files of skills no agent uses; an
agent that needs one again must import it.

## "… may not use the model …"

The agent's `models.allowed` does not include it. Change the list on the *Definition* tab, or ask for an allowed model with `--model`. A route or alias is
checked by the models it would use, so a fallback outside the list is skipped, not tried.

## A skill's script says a module is not found

Scripts run in the sandbox with the standard library only. A skill that needs a package from PyPI cannot run its scripts here; the instructions in its
`SKILL.md` still work.

## An evaluation case fails with "evaluations refuse approvals"

The agent tried a tool that asks first. Evaluations never approve by themselves: list the tool in the case's `approve`, or check that the agent did not need it.
`memory.remember` can never be approved this way.
