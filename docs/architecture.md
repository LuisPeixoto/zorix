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
   - `agents.delegate` is intercepted after the same checks: a `DelegationPlanner` turns the tasks into child executions, the call is marked `Delegated`, the run moves to
     `WaitingForChildren` and the worker is released. When the last child is finished the engine hands the results back, rolls their usage up and re-queues the parent.
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

**One worker at a time.** An engine drives an execution with one worker at a time; a second worker that picks the same execution up (a run answered at once, as it stopped for a person) waits and tries again, so the first one's clean-up cannot release the lease the second holds. Recovery lets go of the lease it held while taking a run over **before** it queues the run, for the same reason.

**A lease is checked when it matters.** A worker renews the lease of its run while it works. Every commit a run makes names the engine that makes it, and the repository refuses it unless that engine still holds the lease, so an engine that lost it cannot mark a tool as started or write anything else. When the renewal finds that another engine holds the lease, or fails for a whole lease, the run is told to stop and writes nothing more: it does not end as cancelled, because the engine that holds the lease carries on from the last checkpoint.
## Security model

See [ADR-0006](adr/0006-permissions-approvals.md) and `SECURITY.md`. In short: deny by default, policy checked when a
tool runs, approvals bound to the exact call, tool output treated as untrusted data, bounded everything, secrets
redacted from events, logs and traces, built-in file tools confined to a workspace root.

## Observability

Every step emits an event and a `tracing` span named after the OpenTelemetry GenAI conventions (`invoke_agent`, `chat`,
`execute_tool`; attributes such as `gen_ai.tool.name` and `gen_ai.tool.call.id`). `ExecutionSummary` reports model,
iterations, tool calls, per-step durations, usage and the result. A `Secret` type and a key-based redaction filter keep
credentials out of everything that is logged or stored.

The spans can be sent to an OpenTelemetry collector you run (**Settings › General › Telemetry**, or `zorix settings set telemetry.endpoint …`): off by default, `https` or a collector on this computer only, spans of the runtime only, with the model, token counts, cost, tool names and outcomes and never a prompt, an answer or an argument; see [ADR-0046](adr/0046-opentelemetry-export.md).

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

## Memory (zorix-memory)

Memory plugs in through the `ContextSource` seam and the tool registry, and nothing in the runtime knows about it. The
`zorix-memory` crate holds retrieval (keyword, vector, fusion and the relevance gate), the recall that is frozen per
execution, the four context sources, the memory tools, the recorder of past work and the folder indexer; `MemoryStack` is the one
wiring both the CLI and the desktop use. The store is behind a `MemoryStore` trait in `zorix-core`, implemented on SQLite in
`zorix-store`, and embeddings are one more method on `ModelGateway`, so any provider can supply them. Token counts are
calibrated from real usage in the runtime. See [memory](memory.md), [knowledge](knowledge.md) and ADR-0013 to ADR-0017.

## Integrations (zorix-mcp, zorix-integrations, zorix-net, zorix-sandbox, zorix-hub)

Everything that reaches outside plugs in through the same `Tool` contract and a registry that can change while ZORIX runs
(`unregister`, `replace`, a `generation` counter). A tool carries its **source** (built in, memory, an MCP server, an
integration, the sandbox) and may describe a call in a sentence for approval cards. An execution **pins** the definition
and hash of every tool it was offered when it starts, so a server changing a tool cannot change a run in progress.

- **`zorix-mcp`** is the MCP client (both protocol eras, stdio and Streamable HTTP, OAuth) with the manager that supervises
  servers, pins their tool definitions and registers approved tools; it is also the MCP server behind `zorix mcp serve`.
- **`zorix-integrations`** holds the native tools and the SSRF-guarded HTTP client they share.
- **`zorix-net`** is the network guard both of them use: which addresses a request that came from a model, a file or a server may
  reach, and how to connect to the address that was checked, so a second DNS answer cannot redirect it.
- **`zorix-sandbox`** runs JavaScript and Python in WebAssembly.
- **`zorix-hub`** assembles them once (store, keychain, registry, manager, sandbox) and turns flags such as `--allow-repo` into
  grants; the CLI and the desktop app both use it.

Servers, tool pins and connections are in SQLite (schema 3); secrets only in the keychain. See [MCP](mcp.md),
[integrations](integrations.md), [sandbox](sandbox.md) and ADR-0018 to ADR-0023.

## Agents (zorix-agents)

An agent is data (a slug and immutable versions of a definition), so the runtime knows nothing about agents beyond a **binding** recorded on the execution
(agent, version, hash, pinned skills, allowed models) and handed to tools. The `zorix-agents` crate holds the **AgentRunner** (how a version and a request
become a `NewExecution`, with grants, limits and memory that can only narrow), the agent file format and its strict YAML subset, the skill library, importer
and tools, the evaluator, and the embedded starter agents; `zorix-hub` wires it to the stores and the registry, and the CLI, the desktop and `zorix mcp serve`
call only the hub. The model allow-list lives in the router (`Router::plan` drops candidates outside it) with a runtime check behind it; live text flows from the
provider adapters through a `TextSink` to a `LiveBus` and `ExecutionEngine::live()`, and is never persisted. See [agents](agents.md), [skills](skills.md),
[evals](evals.md) and ADR-0024 to ADR-0028.

## Orchestration

Delegation lives in three places. `zorix-core` holds the pieces with no I/O: the state and triggers, the `DelegationPlanner` port, the request and result types, the `agent:` access family, and
`Team`. `zorix-runtime` holds the mechanism: intercepting the call, suspending without a worker, waking the parent when its children settle (`reconcile_children`, also run by recovery),
cascading cancel, the depth and descendant guards, and `tree` and `pending_in_tree` for approvals. `zorix-agents` holds the policy (`AgentPlanner`): who may be asked, grants intersected with the
parent's, the budget split, the `<available_agents>` index and the starter team. `zorix-hub` registers the tool and wires the planner. See [orchestration](orchestration.md), [teams](teams.md) and ADR-0029 to ADR-0031.

## Workflows (zorix-workflows)

A workflow is a versioned DAG in `zorix-core` (`workflow.rs`) and `zorix-store` (schema 6). `zorix-workflows` runs it as a client of `ExecutionEngine`: `WorkflowEngine` advances a run one
settled step at a time (agents and teams become ordinary executions through the hub; waits for people and timers are records), and `WorkflowHost` fires schedules, starts workflows for
events, and serves webhooks. It runs in the desktop app or under `zorix serve`. See [workflows](workflows.md).

## Projects (zorix-projects)

Projects and tasks are records in `zorix-core` (`project.rs`) and `zorix-store` (schema 7). `zorix-projects` holds the rules in `ProjectService`: it starts an agent's or a team's task as an
ordinary execution through the hub, reads the run to send the task to Review or mark it failed, holds a run back while a dependency is open, and keeps the history. `reconcile` does that on every
read and on the workflow host's beat (a `Ticker`), so the command line works with no host and the app or `zorix serve` keeps tasks moving. The `tasks.*` and `task.*` tools are registered with
the native tools. See [projects](projects.md) and ADR-0036 and ADR-0037.

## Workforce (zorix-workforce)

Workers, departments, settings, weekly plans and reports and the organisation's history are records in `zorix-core` (`org.rs`) and `zorix-store` (schema 8). `zorix-workforce` holds the rules in
`WorkforceService`: it computes the figures from projects, executions and evaluations, shapes the runs a worker starts (it implements `zorix_projects::RunShaper`, which `ProjectService` asks before
starting a task's or a manager's run) and starts a manager's plan and report. The level itself is applied by the policy engine: `AgentBinding.autonomy` is frozen into the run, and
`DefaultPolicy` answers `Allow`, `AllowUnattended` or `Suggest` besides `Deny` and `RequireApproval`. A `WorkforceTicker` on the host's beat reads finished plans and reports, runs due schedules and
records budget notices. See [workforce](workforce.md) and ADR-0038 and ADR-0039.

## Quality tooling (zorix-bench, fuzz, zorix-sim)

Not part of the product, but part of how it is kept honest ([testing](testing.md), ADR-0042):

- **`zorix-sim`** simulates models: scripted turns, streams that are cut, typed failures (`Failure::Kind`), malformed output, and seeded random faults (`Faulty`). The runtime's own tests use it for every model failure they cover.
- **`FaultyRepository`** (in `zorix-runtime::testing`) wraps any repository and fails the Nth call of a method, before or after the write, so a crash in the middle of a commit can be replayed.
- **`zorix-bench`** builds the scale data set (10,000 executions, about a million events, 200 agents, 5,000 facts, 50,000 pieces of documents) through the real stores and times the paths a person waits on, against `limits.json`.
- **`fuzz/`** is its own workspace with 14 targets for every parser and for the text that wraps untrusted data, with a committed corpus.
- **Error reasons.** A typed error is also a `Reason`: a stable code and the values of its sentence (`zorix_core::Coded`). The command line prints the English; the desktop app sends `{code, args, message}` and the interface writes the sentence from its catalogue (ADR-0040).

## What comes next

| Phase | Plugs into |
|---|---|
| 2 AI Provider Engine | `ModelGateway` implementations, model catalogue, routing |
| 3 Context & Memory ✅ | New `ContextSource`s; memory stores |
| 4 Tool & Integration ✅ | `ToolRegistry` sources: MCP, HTTP, integrations, the sandbox; credentials |
| 5 AI Agent Engine | Agent definitions that create executions with instructions, tools, grants |
| 6 Multi-Agent ✅ | Executions that create child executions through `agents.delegate`; teams |
| 7 Workflows | A scheduler of executions with conditions; uses events as triggers |
| 8–9 Tasks, Workforce | Clients of `ExecutionEngine` |
| 10 Command Center | The full interface over events and queries |
| 11 Marketplace | Packaging of agents, skills and workflows |
