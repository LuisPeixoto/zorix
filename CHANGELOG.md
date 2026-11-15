# Changelog

All notable changes are documented here. The format follows [Keep a Changelog](https://keepachangelog.com) and the
project follows [Semantic Versioning](https://semver.org).

## [Unreleased]

## [0.1.0] - 2026-11-15

The first release: the **Harness Runtime**, a command line and a minimal desktop app. No real model provider yet; the
runtime is driven by simulated models.

### Added

- **Executions** with a strict, validated lifecycle (`created`, `queued`, `running`, `waiting_for_model`,
  `waiting_for_tool`, `waiting_for_approval`, `paused`, `completed`, `failed`, `cancelled`).
- **The execution loop**: context building from ordered, budgeted sources; model calls behind a provider-neutral
  `ModelGateway`; retries with backoff; timeouts; limits on iterations, tool calls, tokens, cost and working time;
  cancel, pause and resume; validation of final answers, including structured output against a JSON Schema.
- **Tools**: an MCP-shaped contract, a registry, and an executor with schema validation, timeouts and output caps.
- **Permissions and policy**: deny-by-default grants, a policy engine evaluated when a call runs, and **human approval**
  that suspends an execution and is bound to the exact call it names.
- **Prompt-injection containment**: tool output is labelled untrusted and cannot change grants or policy.
- **Events**: an ordered, versioned, persisted stream that can be replayed and followed live without gaps.
- **Persistence and recovery** on SQLite: atomic commits, checkpoints, leases, idempotent creation, and crash recovery
  that never repeats a finished tool and asks a person before repeating one that is not safe to repeat.
- **Observability**: trace spans named after the OpenTelemetry GenAI conventions, secret redaction, and an execution
  summary with step timings and usage.
- **Simulated models**: scripted, failure-injecting and a rule-based demo model.
- **Built-in demo tools**: clock, calculator, text statistics, a per-tenant notebook, and file tools confined to a workspace.
- **`zorix` command line**: `run`, `list`, `show`, `events`, `approve`, `deny`, `cancel`, `resume`, `recover`, `tools`,
  `data-dir`, with JSON output and exit codes.
- **Desktop app** (Tauri 2, Svelte 5): runs list, new run form, live run view with an approval card, pause, resume and cancel.
- CI on Linux, macOS and Windows, and a release workflow for installers, command line archives and checksums.
- Documentation, ten architecture decision records, and a comparison of the agent runtimes studied.

### Known limits

- Installers are **not signed or notarized**, and there are no automatic updates yet.
- File tools confine paths; there is no sandbox for tools that run code, and no such tool exists yet.
- Token counts are estimates until real providers arrive.
- One program at a time per data folder; no accounts or shared store.
