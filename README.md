<p align="center"><img src="docs/images/icon.png" width="96" alt="ZORIX logo" /></p>

<h1 align="center">ZORIX</h1>

<p align="center"><strong>A harness for AI.</strong> Run activities with explicit permissions, human approval, limits,<br/>
checkpoints and a complete record, whichever model does the thinking.</p>

<p align="center">
  <a href="https://github.com/LuisPeixoto/zorix/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/LuisPeixoto/zorix/actions/workflows/ci.yml/badge.svg" /></a>
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue.svg" /></a>
  <img alt="Rust and Tauri" src="https://img.shields.io/badge/Rust%20%2B%20Tauri-macOS%20%C2%B7%20Windows%20%C2%B7%20Linux-5b4bdb" />
</p>

![ZORIX waiting for your approval before a tool changes anything](docs/images/ui-approval.png)

> **Status: early development.** Phase 1 (the **Harness Runtime**) and Phase 2 (the **AI Provider Engine**) are done: ZORIX
> runs on Anthropic, OpenAI, Gemini, OpenAI-compatible servers and local Ollama models. Memory, agents and workflows come in
> later phases (see the [roadmap](docs/roadmap.md)). **A cloud provider receives the conversation of a run**; use
> `--local-only` to keep everything on your computer.

A model can be powerful, but the real strength is in **integration, orchestration and control**. ZORIX is the
infrastructure under that: it decides what a model may see, what it may do, when a person must say yes, and it records
everything. It is not tied to one model or provider, and the core does not know which one is behind it.

## Highlights

- **Nothing is allowed by default.** An execution has explicit grants. A tool the model was never given is refused even
  if the model asks for it, and a prompt injected through a tool result cannot unlock anything.
- **Human in the loop that cannot be forged.** A call that changes something suspends the execution until a person answers.
  The answer is bound to the exact tool and arguments (by hash), can be given once, and survives closing the program.
- **Survives crashes.** State is saved after every step in SQLite. A stopped execution continues from its checkpoint; a
  finished tool is never run twice, and one that cannot safely be repeated asks you first.
- **Everything is recorded.** Ordered, replayable events; a summary of model, tools, timings and usage; trace spans that
  follow the OpenTelemetry GenAI names; secrets redacted from events and logs.
- **Model agnostic.** The loop talks to a small `ModelGateway` contract. [Anthropic, OpenAI, Gemini, compatible servers and
  Ollama](docs/providers.md) plug in behind it; keys stay in your system keychain; routes fall back when a provider fails.
- **One code base, three systems.** Rust core, Tauri shell, tested on macOS, Windows and Linux.
- **Bounded.** Iterations, tool calls, tokens, cost, time, output size: every limit is explicit and fails with a clear reason.

## Screenshots

| | |
|---|---|
| ![New run](docs/images/ui-new-run.png) | ![Approval card](docs/images/ui-approval.png) |
| Choose what a run may use. Anything that changes something says it asks first. | The tool, the exact arguments, why you are asked, and a fingerprint of the call. |

## What is in Phases 1 and 2

| Area | What it does | Guide |
|---|---|---|
| Execution engine and lifecycle | Create, start, pause, resume, cancel, recover. Ten states, validated transitions | [Runtime](docs/runtime.md) |
| The loop | Context, model call, tool calls, validation, limits, retries, timeouts | [Runtime](docs/runtime.md) |
| Context builder | Ordered, budgeted sources; extensible for memory later | [ADR-0007](docs/adr/0007-context-builder.md) |
| Tools | MCP-shaped contract, schema validation, timeouts, output caps, eight demo tools | [Tools](docs/tools.md) |
| Permissions and policy | Grants, rules, approvals, prompt-injection containment, tenants | [Permissions](docs/permissions.md) |
| Events | Versioned, persisted, replayable, followed live | [Events](docs/events.md) |
| Persistence | SQLite, atomic commits, checkpoints, leases, idempotency, recovery | [Persistence](docs/persistence.md) |
| Observability | Spans, redaction, execution summary | [Permissions](docs/permissions.md#credentials) |
| Command line | `zorix run`, `approve`, `events`, `recover`… | [CLI](docs/cli.md) |
| Desktop app | Runs, new run, live view, approval card, providers and keys, model picker | [Desktop](docs/desktop.md) |
| Providers | Anthropic, OpenAI, Gemini, compatible servers, Ollama; keychain keys; routes and fallback | [Providers](docs/providers.md) |

## Install

Prebuilt installers will be attached to each [release](https://github.com/LuisPeixoto/zorix/releases). The first releases
are not signed (see [distribution](docs/distribution.md)). To build from source you need Rust 1.90 or newer and, for the
desktop app, Node.js 22 or newer:

```bash
git clone https://github.com/LuisPeixoto/zorix.git && cd zorix
cargo build --release -p zorix-cli        # target/release/zorix
cd apps/desktop && npm install && npm run tauri build
```

## Quick start

```bash
export ZORIX_HOME=$(mktemp -d)/zorix                 # a throwaway data folder

zorix run "What is 17*23 and save it as result" \
  --allow math.evaluate --allow notes.put --approve never
# runs math.evaluate, then stops: notes.put changes something and asks first (exit code 3)

zorix list
zorix approve <execution-id>                          # from another process; continues from the checkpoint
zorix events  <execution-id>                          # the complete, ordered record
```

With a real model (the conversation goes to that provider, or stays here with Ollama):

```bash
zorix provider set-key anthropic                      # hidden prompt; kept in the system keychain
zorix run "What is 17*23?" --allow math.evaluate      # uses Anthropic when it has a key, else the demo model
zorix run "What time is it?" --allow clock.now --model ollama/qwen3:1.7b --local-only
```

More in [getting started](docs/getting-started.md). To use the runtime from Rust:

```bash
cargo run -p zorix-runtime --example embed
```

## Documentation

| Guide | |
|---|---|
| [Getting started](docs/getting-started.md) | Build it, run the first activity |
| [The runtime](docs/runtime.md) | Executions, lifecycle, loop, limits, failures |
| [Permissions, policy and approvals](docs/permissions.md) | What an AI may do, and how a person decides |
| [Tools](docs/tools.md) | The contract, the built-in tools, writing your own |
| [Events](docs/events.md) | Every event and how to follow them |
| [Persistence and recovery](docs/persistence.md) | What is saved, what happens after a crash |
| [Providers, models and routing](docs/providers.md) | Anthropic, OpenAI, Gemini, Ollama; keys; routes and fallback |
| [Command line](docs/cli.md) | Every command, option and exit code |
| [Desktop app](docs/desktop.md) | The Tauri app |
| [Architecture](docs/architecture.md) | Layers, contracts, decisions; the [ADRs](docs/adr) |
| [Technology comparison](docs/research/runtime-landscape.md) | LangGraph, OpenAI Agents SDK, ADK, Temporal, MCP and others: what we took, what we built |
| [Testing](docs/testing.md) | How each required case is proven |
| [Distribution](docs/distribution.md) | Installers, signing, releases |
| [Troubleshooting](docs/troubleshooting.md) | Symptom, cause, fix |
| [Roadmap](docs/roadmap.md) | The eleven phases |

## Project layout

```
crates/
  zorix-core      model and contracts: lifecycle, permissions, events, the traits everything depends on (no I/O)
  zorix-runtime   the engine, loop, tools, policy, context, events, in-memory repository
  zorix-store     SQLite persistence and the data folder lock
  zorix-sim       simulated models: scripted, failure-injecting, rule-based demo
  zorix-tools     built-in demonstration tools
  zorix-cli       the `zorix` command line
apps/desktop      Tauri 2 shell (src-tauri) and the Svelte 5 interface (src)
docs/             guides, architecture, decision records, research
```

## Contributing

Bug reports, documentation, tests and code are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) and the
[code of conduct](CODE_OF_CONDUCT.md). Report vulnerabilities privately, as described in [SECURITY.md](SECURITY.md).

## License

[MIT](LICENSE)
