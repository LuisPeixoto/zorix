# Runtime landscape

What existing agent and durable-execution tools do, what ZORIX takes from each, and why the runtime core is built
in-house. Researched in November 2026 from the official documentation of each project; items marked *(verify)* came
from secondary sources or from pages whose date could not be confirmed.

## How to read this

The questions are the ones the Phase 1 brief asked about every solution: how it manages executions, context, tool
calls, loops, errors and retries, state, interruption and resume, security, and what its limits are.

## Comparison

| | Executions and loop | Context | Tool calls | Errors, retries | Persistence | Pause / resume | Security and permissions | Limits for ZORIX |
|---|---|---|---|---|---|---|---|---|
| **LangGraph** | Graph of nodes; a run is a *thread* advanced in supersteps | State object (reducers) plus message list | Tool nodes; model decides | Retry policy per node | Checkpointer saves state after each step; three durability modes (exit, async, sync) | `interrupt()` pauses; resume with a `Command` value; the node restarts from its beginning | Left to the app | Python/JS. A node that was interrupted re-runs, so side effects must be idempotent. The graph is a workflow concern (Phase 7) |
| **LangChain** | Agent loop on top of LangGraph | Message history, middleware | Tool decorators, structured output | Model/tool retry middleware | Via LangGraph | Via LangGraph interrupts | Guardrail middleware | Library of integrations more than a runtime. Useful as a catalogue of providers later |
| **LlamaIndex Workflows** | Event-driven steps, async | `Context` object, serializable (`to_dict`/`from_dict`) | Agent tools | Step retries | Context must be saved by the app *(verify)* | `InputRequiredEvent` / `HumanResponseEvent` | Left to the app | Python. Persistence is manual |
| **Microsoft Agent Framework 1.0** | Agents and graph workflows | Sessions, context providers | Function tools, MCP | Middleware | Checkpoints at superstep boundaries; file and Cosmos storage; Durable Task package still prerelease *(verify)* | Request/response ports for humans | Middleware, approvals | .NET/Python |
| **OpenAI Agents SDK** | `Runner` loop: model, tools, handoffs, until final output or max turns | Sessions, run context | Function tools, hosted tools, MCP | Max turns, error handlers | Sessions; run state can be serialized | Tool `needs_approval` (bool or predicate) interrupts the run; resume with decisions | Input, output and tool guardrails (separate from approvals) | Python/JS, built around one vendor's API shape. A guardrail only protects the paths it sees |
| **Google ADK** | Runner over agents; everything is an `Event` | Session = events + state (`temp:` keys not persisted) | Function tools, MCP | Callbacks | Session services (memory, DB) | Tool confirmation request/response events | Callbacks | Python/Java/Go. Open report (adk-java #1389): the resume logic looks for the approval among *all* session events, so a user-authored event could forge it |
| **Mastra** | Agents and step workflows | Memory, working memory | Tools with zod schemas, MCP | Step retries | Snapshots in a storage provider | `suspend()` with a `resumeSchema`; `run.resume({step, resumeData})` | Left to the app | TypeScript. Durable agents are beta *(verify)*. Has a Temporal integration |
| **Temporal** | Workflow history + activities, replayed on any worker | Whatever the workflow holds | Activities | Activity retry policy, timeouts | Event history in the Temporal service | Signals, updates, timers; replay rebuilds state | Namespaces, mTLS | The **Rust SDK is GA (Sept 2026)**, but it needs a Temporal service; too heavy for a desktop app. Its ideas (history, idempotent activities) shape our repository contract |
| **Inngest** | Functions made of durable `step.run` steps | Per function | Tools as steps | Per-step retry budget with backoff | Step results memoized by the service | `waitForEvent`, sleeps | Service-level | Hosted or self-hosted server. AgentKit is TypeScript |
| **MCP (2026-07-28)** | Protocol, not a runtime | Resources, prompts | `tools/list`, `tools/call` with `inputSchema`, `outputSchema`, annotations | JSON-RPC errors; `isError` results | None | Tasks extension (polling `tasks/get`/`tasks/update`); multi round-trip requests for elicitation | Authorization spec, tool annotations (hints only) | Only the tool shape is adopted now; an MCP client is Phase 4 |
| **Rig (Rust, 0.4x)** | Agent builder with `max_iterations`; serializable `AgentRun` *(verify)* | Preamble, dynamic context | Typed tools, schema from Rust types | Provider errors | Not a goal | Not a goal | Not a goal | A good provider layer candidate for Phase 2, not a governed runtime |
| **OpenTelemetry GenAI** | Naming conventions | n/a | `execute_tool` span, `gen_ai.tool.name`, `gen_ai.tool.call.id` | `error.type` | n/a | n/a | Content capture is opt-in | Still marked *development*; names may change |

## What ZORIX takes

1. **A checkpoint after every step**, keyed by an execution id (LangGraph, Inngest). Resume continues from the last
   checkpoint; steps that already have a recorded result are not repeated.
2. **Approval as suspension** (OpenAI, ADK, Mastra, LangGraph): the execution stops in `WAITING_FOR_APPROVAL`, the
   process may exit, and a later decision resumes it. We add what ADK lacks: the approval is bound to a
   runtime-issued id *and* the hash of the exact tool call.
3. **Idempotency discipline** (Temporal, LangGraph docs): a tool that started but did not record a result is re-run only
   if it is declared idempotent; otherwise a human decides.
4. **Tool contract shaped like MCP**: name, description, input and output schema, annotations (read-only,
   destructive, idempotent, open-world). Phase 4 can then expose MCP tools with no translation layer.
5. **Per-step retry budgets** (Inngest) separated from loop limits.
6. **Guardrails separate from approvals** (OpenAI): policy decides allow / deny / ask; validators check results.
7. **Event stream as the public face** (ADK): every state change is an ordered, replayable event.
8. **GenAI span names** (OpenTelemetry) emitted through `tracing`, so an OTLP exporter can be added without changing
   the runtime.

## What ZORIX builds itself, and why

| Component | Reason |
|---|---|
| Lifecycle state machine | It is small, central and must be provably strict. No framework exposes the states we need (approval, pause, model wait, tool wait) as a public contract |
| Execution loop | Must be provider-independent and enforce policy at *execution* time. Framework loops put the model in charge of the control flow |
| Permission and policy engine | No tool covers deny-by-default grants, per-call approval binding and limits together |
| Repository + checkpoints | A desktop app needs an embedded store. SQLite fits; the trait keeps Postgres or Temporal possible |
| Context builder | The shape of our context (provenance-tagged, budgeted, extensible with memory in Phase 3) is product-specific |

## What ZORIX reuses

`tokio` (async runtime, cancellation tokens through `tokio-util`), `serde`/`serde_json`, `jsonschema` (input and output
validation), `rusqlite` with bundled SQLite, `tracing`, `clap`, `uuid` (v7 ids), `sha2`, `thiserror`.

## Not adopted yet

- **Temporal / Inngest backends**: needs a server; revisit for a team or cloud edition (the repository and scheduler
  traits are the seam).
- **MCP client**: Phase 4.
- **Provider SDKs / Rig**: Phase 2.
- **OTLP export**: after the GenAI conventions stabilise.

## Sources

- LangGraph: interrupts, checkpointers and durable execution pages at docs.langchain.com/oss/python/langgraph
- OpenAI Agents SDK: "Guardrails and human review", developers.openai.com/api/docs/guides/agents/guardrails-approvals
- Google ADK: tool confirmation, sessions and state; issue google/adk-java#1389
- Microsoft Agent Framework: learn.microsoft.com/agent-framework/workflows/checkpoints
- Mastra: suspend and resume, mastra.ai/docs/workflows/suspend-and-resume
- LlamaIndex: workflows, human in the loop, developers.llamaindex.ai
- Temporal: Rust SDK general availability announcement (September 2026), temporal.io/blog
- Inngest: AgentKit and durable steps, inngest.com/docs
- MCP: specification 2026-07-28 and changelog, modelcontextprotocol.io/specification/2026-07-28/changelog
- Rig: docs.rs/crate/rig
- OpenTelemetry GenAI semantic conventions, github.com/open-telemetry/semantic-conventions-genai
