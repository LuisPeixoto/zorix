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
`ZORIX_LOG=debug` (desktop) or `RUST_LOG=debug` for logs; prompts, arguments and results are never written to logs.
