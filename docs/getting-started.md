# Getting started

ZORIX runs on macOS, Windows and Linux from one code base. This guide builds it from source and runs the first activity.
Prebuilt installers will be attached to each [release](https://github.com/LuisPeixoto/zorix/releases) when the first one is published.

## Requirements

- [Rust](https://rustup.rs) stable, 1.90 or newer
- [Node.js](https://nodejs.org) 22 or newer (desktop app only)
- Linux only, for the desktop app: the Tauri system packages

```bash
sudo apt-get install -y libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf
```

## Build and test

```bash
git clone https://github.com/LuisPeixoto/zorix.git
cd zorix
cargo test --workspace --exclude zorix-desktop    # 247 tests, no network, no provider needed
```

## Your first run (command line)

Use a throwaway data folder so nothing touches your real data:

```bash
export ZORIX_HOME=$(mktemp -d)/zorix
cargo run -q -p zorix-cli -- run "What is 17*23 and save it as result" \
  --allow math.evaluate --allow notes.put --approve never
```

Nothing is allowed unless you allow it (`--allow`). The built-in demo model asks for `math.evaluate`; it runs. Then it
asks for `notes.put`, which changes something, so the execution **stops and waits for you** (exit code 3):

```
#13  tool.requested         notes.put {"key":"result","value":"17*23 = 391"}
#14  waiting_approval       notes.put: notes.put asks for approval on every call.

  It is waiting. Answer with:
    zorix approve <execution-id> <approval-id>
```

The process has ended; the state is on disk. Answer from another command:

```bash
cargo run -q -p zorix-cli -- list
cargo run -q -p zorix-cli -- approve <execution-id>     # runs the tool and finishes
cargo run -q -p zorix-cli -- show <execution-id>        # model, tools, timings, usage, result
cargo run -q -p zorix-cli -- events <execution-id>      # the full ordered record
```

Or answer on the spot with `--approve ask` (it asks on the terminal), and refuse with `zorix deny <id> --note "not now"`.
See the [command line reference](cli.md).

## Your first run (desktop app)

```bash
cd apps/desktop
npm install
ZORIX_HOME=/some/throwaway/folder npm run tauri dev
```

Pick a tool set, describe the activity, and start it. When a tool needs your approval a card shows the tool, the exact
arguments and why you are being asked. See the [desktop guide](desktop.md).

## Use the runtime from Rust

```bash
cargo run -p zorix-runtime --example embed
```

The example is about 80 lines: a tool, a model, a grant and a human approval, all in memory. See [the runtime](runtime.md).

## Where to go next

| I want to… | Read |
|---|---|
| Understand what an execution is and how it ends | [The runtime](runtime.md) |
| Control what an AI may do | [Permissions, policy and approvals](permissions.md) |
| Add a tool | [Tools](tools.md) |
| Follow executions from my own code | [Events](events.md) |
| Know what happens after a crash | [Persistence and recovery](persistence.md) |
| See the design decisions | [Architecture](architecture.md) and the [ADRs](adr) |
