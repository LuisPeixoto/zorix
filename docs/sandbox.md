# The code sandbox

`code.run` lets a run execute a short JavaScript or Python program and read what it printed. The program runs in a
**WebAssembly** sandbox ([wasmtime](https://wasmtime.dev)): it has **no network**, no access to your files unless you allow it,
and hard limits. The design is in ADR [0022](adr/0022-wasm-sandbox.md).

| | JavaScript | Python |
|---|---|---|
| Runtime | QuickJS, built into ZORIX (about 1.5 MB) | CPython 3.12 for WASI, downloaded once (about 26 MB) and checked against a fixed SHA-256 |
| Ready | always | after `zorix sandbox install python`, or **Install Python** in Integrations › Connections |

Grant it with `--allow-code javascript` or `--allow-code python` (or the chips in the New run form).

## Limits

| Limit | Value | What happens |
|---|---|---|
| Memory | 256 MB | the program is stopped with a clear reason |
| Time | 10 seconds | stopped (a program that never yields is interrupted too) |
| Output | 1 MB each of standard output and error | stopped when full |
| Scratch files | 50 MB, in a folder made for the run | stopped; the folder is deleted afterwards |
| Network | none | there is no socket to open |

Every stop says why (`time`, `memory`, `output`, `disk`, `cancelled`) and what was printed until then is returned.

## Files

By default a program sees nothing of yours; it can write inside its scratch folder at `/work`, and nothing is left for the next
run. `code.run_in_workspace` also mounts the run's workspace folder **read-only** at `/workspace`; it needs `read` on
`workspace:*` as well as the language grant, and **asks first**, because the program can read your files. It can never write
them.

**What is tested.** Every system, in CI, runs escape tests (`crates/zorix-sandbox/tests/escapes.rs`): paths that could name the host (`..`,
backslashes, drive letters, `\\?\` and UNC forms, `/proc`, `/etc`), a mount that tries to take the place of `/work` or `/workspace`,
writes and removals where only reading was allowed, and, on macOS and Linux, links inside a folder the program was shown and links the
program makes itself. A sandbox escape on Windows was fixed in a dependency in 0.9.x; those paths are what such an escape uses, so the
same list runs on the Windows runner.

## What it is for, and not

Calculations, parsing, reshaping text and data the model would otherwise get wrong. It is not a place for long-running work,
packages (there is no `pip` and no network), or anything that needs a GUI. Compiled modules are cached next to the runtime so
later runs start quickly.

## Commands

```text
zorix sandbox status            # what is ready, and the limits
zorix sandbox install python    # download CPython (about 26 MB), verify, keep
```
