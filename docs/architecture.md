# Architecture

ZORIX is a desktop platform for orchestrating AI models, tools and (later) agents. **Phase 1 builds the Harness
Runtime**: the core that runs one AI-driven activity from request to result under control. Everything later (providers,
memory, tools over MCP, agents, workflows, tasks, the full interface) is a client or a plug-in of this core.

## Layers

```
Frontend (Svelte)        Tauri runtime (commands, Channel events)        CLI
        └──────────────────────────┬─────────────────────────────────────┘
                                   ▼
                          zorix-runtime  ── Engine (ExecutionEngine)
        ┌────────────┬─────────────┼───────────────┬──────────────┬────────────┐
   Loop + retries   Context     ToolExecutor   PolicyEngine    EventBus     Scheduler
                    Builder     + registry     (grants, limits, (broadcast)  (local)
                                               approvals)
        └────────────┴─────────────┼───────────────┴──────────────┴────────────┘
                                   ▼
                              zorix-core   (contracts + model, no I/O)
                                   ▲
        zorix-store (SQLite)   zorix-sim (simulated models)   zorix-tools (demo tools)   [Phase 2+: providers, MCP]
```

- **Rust core** owns every rule that needs consistency or safety. The frontend never decides permissions or state.
- **Tauri runtime** is a thin layer: typed commands in, events out through a `Channel`.
- **External services** (providers, tools) are always behind a trait.

## One execution, step by step

An *execution* is one run of one activity. A conversation may own many executions; they share nothing except what the
client passes in.

1. `Engine::create(NewExecution)` validates input, applies the idempotency key (same key → same execution), stores
   `Created`, publishes `execution.created`.
2. `Engine::start(id)` moves it to `Queued`; the scheduler hands it to a worker, which takes a lease and moves it to
   `Running`.
3. **Loop.** Each iteration: build context → checkpoint → call the model (`WaitingForModel`) → classify the answer.
   - *Final answer*: run the `ResultValidator`s; success → `Completed`; rejected → feed the reason back (bounded) or
     `Failed`.
   - *Tool calls*: for each call, resolve the tool, validate arguments against its JSON Schema, ask the policy engine
     **now** (`Allow` / `Deny` / `RequireApproval`), then run it (`WaitingForTool`) with timeout and output cap. The
     result is appended as untrusted data and the loop continues.
   - `RequireApproval` persists an approval bound to the call hash and moves to `WaitingForApproval`. The worker
     releases the execution. `Engine::approve/deny` later re-queues it.
4. Limits are checked every iteration (iterations, tool calls, tokens, cost, wall-clock). Exceeding one fails the
   execution with a specific error and a final event.
5. Cancellation (`CancellationToken`) is honoured at every await; pause takes effect at the next step boundary.

## Contracts (zorix-core)

Each trait is one responsibility with a reason to exist.

| Trait | Responsibility | Why it is a trait |
|---|---|---|
| `ExecutionEngine` | The operations clients call: create, start, get, subscribe, cancel, pause, resume, approve, deny, recover | The CLI, the desktop shell and later modules (workflows, agents, tasks) depend on this contract, not on the runtime crate |
| `ModelGateway` | One model call: `ModelRequest` → `ModelResponse` (text or tool calls, usage) | Phase 2 adds providers; tests and demos use the simulator |
| `ContextBuilder` / `ContextSource` | Turn an execution view into a model request, from ordered, budgeted sources | Phase 3 adds memory and retrieval as sources |
| `ToolRegistry` | Register, list and look up tools | Phase 4 adds MCP and integrations |
| `Tool` (executor side) | `ToolSpec` + `call(ctx, args)` | Each tool is independent; the engine validates and executes uniformly (`ToolExecutor` is a concrete helper, not a trait) |
| `PolicyEngine` | Decide Allow / Deny / RequireApproval for a call, and check limits | Rules will come from users and organisations later. *The brief's "PermissionEngine" is the grant check inside it: separating them would add a contract with no second responsibility* |
| `EventPublisher` | Publish a committed event to subscribers | The in-process bus today; a persisted fan-out or remote stream later |
| `ExecutionRepository` | Persist executions, events, checkpoints, tool calls, approvals, leases, idempotency keys | In-memory for tests, SQLite now, others later; shared contract tests |
| `ExecutionScheduler` | Queue executions and hand them to workers with a lease | Local tokio scheduler now; a distributed queue later |
| `ResultValidator` | Accept or reject a final answer, with a reason | Quality evaluation grows in later phases |
| `Clock` | Current time | Deterministic tests of timeouts and leases |

Not traits (yet, on purpose): the loop, the retry policy and the redaction filter. They have one implementation and no
second consumer.

## Persistence and recovery

SQLite through `ExecutionRepository`. One transaction per state change writes: the execution row (with `version`
check), the new events, and a checkpoint when the step defines one. A checkpoint holds the transcript, iteration, pending
tool calls, usage and the next action. `Engine::recover()` runs at start-up:

| Found state | Action |
|---|---|
| `Queued`, `Running`, `WaitingForModel` | Re-queue; the loop resumes from the last checkpoint (the model call is repeated; it has no side effect) |
| `WaitingForTool`, call recorded as started without a result | Idempotent tool → run again. Otherwise → `WaitingForApproval` with the reason "outcome unknown" so a person decides |
| `WaitingForApproval`, `Paused` | Left as they are |
| Expired leases | Released |

## Security model

See [ADR-0006](adr/0006-permissions-approvals.md) and `SECURITY.md`. In short: deny by default, policy checked when a
tool runs, approvals bound to the exact call, tool output treated as untrusted data, bounded everything, secrets
redacted from events, logs and traces, built-in file tools confined to a workspace root.

## Observability

Every step emits an event and a `tracing` span named after the OpenTelemetry GenAI conventions (`invoke_agent`, `chat`,
`execute_tool`; attributes such as `gen_ai.tool.name` and `gen_ai.tool.call.id`). `ExecutionSummary` reports model,
iterations, tool calls, per-step durations, usage and the result. A `Secret` type and a key-based redaction filter keep
credentials out of everything that is logged or stored.

## Cross-platform rules

One code base. Crates other than the desktop shell have no platform code and run their tests on macOS, Windows and Linux
in CI. The data folder comes from one function (`data_dir()`), overridable with `ZORIX_HOME`. File tools use
`std::path` and are tested with Windows-style inputs. Anything that will need a platform feature later (keychain,
sandbox) goes behind a trait in its own module.

## Providers (zorix-providers)

The runtime sees one `ModelGateway`. Behind it a **router** resolves `provider/model`, aliases and routes, filters and orders
candidates, falls back, and records which model answered. One adapter per API family (Anthropic, OpenAI Responses,
Chat Completions, Ollama, Gemini) sits on a shared HTTP client (HTTPS, size and idle limits, no redirects, secret headers)
and streaming parsers. A catalogue knows windows, capabilities and prices; a secret store reads keys when a call is made.
See [providers](providers.md) and ADR-0009 to ADR-0012.

## What comes next

| Phase | Plugs into |
|---|---|
| 2 AI Provider Engine | `ModelGateway` implementations, model catalogue, routing |
| 3 Context & Memory | New `ContextSource`s; memory stores |
| 4 Tool & Integration | `ToolRegistry` sources: MCP, HTTP, integrations; credentials |
| 5 AI Agent Engine | Agent definitions that create executions with instructions, tools, grants |
| 6 Multi-Agent | Executions that create child executions (parent id is already part of the model) |
| 7 Workflows | A scheduler of executions with conditions; uses events as triggers |
| 8–9 Tasks, Workforce | Clients of `ExecutionEngine` |
| 10 Command Center | The full interface over events and queries |
| 11 Marketplace | Packaging of agents, skills and workflows |
