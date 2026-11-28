# The runtime

The Harness Runtime runs **one activity from request to result, under control**. It does not care which model thinks,
which interface asks, or which tools exist. This guide explains what it does and the rules it keeps. For the design
reasons see [architecture](architecture.md) and the [ADRs](adr).

## What an execution is

An *execution* is one run of one activity, with an id, a state, limits, grants and a history. It is **not** a
conversation: a conversation can start many executions, and they share nothing except what the client passes in.

```rust
let created = engine.create(NewExecution { request, grants, limits, ..Default::default() }).await?;
engine.start(&tenant, created.id).await?;          // or engine.submit(...) for both
let done = engine.settle(&tenant, created.id).await?; // wait until it finishes, waits for a person, or pauses
```

A runnable version is in [`crates/zorix-runtime/examples/embed.rs`](../crates/zorix-runtime/examples/embed.rs).

## The lifecycle

```
Created ──► Queued ──► Running ◄──► WaitingForModel
   │           │          │  ▲ ◄──► WaitingForTool
   │           │          │  ├────► WaitingForApproval
   │           │          │  └────► WaitingForChildren
   │           │          ├──────► Paused ──► Queued
   ▼           ▼          ▼
Cancelled   Cancelled   Completed | Failed | Cancelled      (terminal)
```

| State | Meaning |
|---|---|
| `created` | Stored, not asked to run |
| `queued` | Waiting for a worker |
| `running` | A worker is deciding the next step |
| `waiting_for_model` | A model call is in flight |
| `waiting_for_tool` | A tool call is in flight |
| `waiting_for_approval` | Suspended until a person decides. No worker holds it; the process may exit |
| `waiting_for_children` | The run asked other agents (`agents.delegate`) and waits for all of them. No worker holds it; it is queued again when the last child is finished. See [orchestration](orchestration.md) |
| `paused` | Stopped on request; resumable |
| `completed`, `failed`, `cancelled` | Finished. Nothing can change them |

An execution also records where it sits in a tree: `parent` (the run that asked), `root` (the run at the top) and `depth` (0 for a root). `list` can return
roots only or the children of one run, and `Engine::tree` returns a root with everything below it ([orchestration](orchestration.md)).

Every change goes through one pure function, `transition(state, trigger)`, and the repository refuses a write made from
an old version, so two workers can never move the same execution. `INITIALIZING` from the brief is not a state: building
the context is a step inside `running`. Retrying is a counter on a step, not a state.

## The loop

Each iteration:

1. **Check** shutdown, cancellation, pause and the limits.
2. If tool calls from the last model answer are pending, handle the next one (below). Otherwise:
3. **Build the context** from sources (system text, the task and the files attached to it, history, extra sources) within a token budget. Attached files are untrusted `<document>`s after the task, never dropped, and cut in proportion when together they would take more than half the model's window ([ADR-0049](adr/0049-files-attached-to-a-run.md)). When the history no longer fits, the oldest turns are **summarised** (one extra model call, counted in the run's usage and limits, off with `zorix settings set context.summaries off`) and the summary stands where they were, as untrusted data; if that cannot be done they are left out with a note ([ADR-0043](adr/0043-summaries-of-long-conversations.md)).
4. **Call the model** through `ModelGateway`, with a timeout. Temporary errors are retried with backoff.
5. If the model answered with text only, **validate** it. If it asked for tools, remember the calls and checkpoint.

A tool call goes through, in order: *find the tool* → *validate the arguments against its JSON Schema* → *ask the policy
engine, now* → *suspend for approval if needed* → *run with a timeout and an output cap* → *add the result to the
context as untrusted data* → *checkpoint*.

Nothing here knows a provider. The loop talks to the `ModelGateway` trait; Phase 2 adds real providers behind it.

## Limits

Every execution has limits. They are checked on every iteration, and exceeding one fails the execution with a message
that names it.

| Limit | Default | Meaning |
|---|---|---|
| `max_iterations` | 12 | Model steps |
| `max_tool_calls` | 24 | Tool calls requested |
| `max_total_tokens` | none | Input plus output tokens (an estimate in this phase) |
| `max_cost_micros` | none | Cost, in millionths of a dollar. A model whose price is not known cannot keep it: the run is refused before anything is sent ([providers](providers.md#models-and-numbers)) |
| `max_active_ms` | 300 000 | Time spent working. Waiting for a person is **not** counted |
| `model_timeout_ms` | 60 000 | The longest one model call may go **without progress**: from the start to the first piece of text, and between pieces. An answer that keeps arriving is not cut however long it is; one that goes quiet is, after this long |
| `max_model_attempts` | 3 | Tries for one model call that fails with a temporary error |
| `max_validation_retries` | 1 | Times a rejected final answer goes back to the model |

A call that crosses a token or cost limit completes; the execution stops right after it.

Whatever the limit, one model call ends after 600 000 ms in all (the longest the providers' HTTP clients wait). Progress is text reaching the live sink, so a response that is only a tool call, which streams no text, has the whole `model_timeout_ms` to arrive.

## Failures and retries

| What went wrong | What happens |
|---|---|
| Model: temporary error or timeout | Retried with backoff (doubling, capped, with jitter, honouring `retry_after`), up to `max_model_attempts`; each retry is an event |
| Model: permanent error, or a provider error that cannot succeed (bad credentials, a context that is too long, unknown model, a refusal reported as an error) | The execution fails with the provider's own sentence, without a retry |
| Model: the stream is cut | The attempt is retried, and readers of the live text are told to forget what it wrote (a `reset`), so the answer is never shown twice |
| Model: it declines (finish reason `refusal`) | The execution fails with "The model declined to answer", and what it said is not the result |
| Model: the answer ran out of room (finish reason `length`) | Sent back once with the reason, then the execution fails; a cut-off answer is never the result |
| Tool: invalid arguments, unknown tool, policy denial, tool error | The model receives an error result and can adapt. The execution does not fail |
| Tool: temporary error | Retried (up to 3 starts) if the tool says so. A *timeout* is retried only for tools declared idempotent |
| Final answer rejected by a validator | Sent back once with the reason, then the execution fails |
| A limit is reached | The execution fails, naming the limit |

## Cancel, pause, resume

- `cancel` stops a model call or a tool call in flight, records the reason, and closes any open approval.
- `pause` takes effect at the next step boundary. A paused execution keeps its checkpoint. `resume` queues it again.
- Pausing an execution that is waiting for approval is refused: it is already suspended.
- Closing the program (`shutdown`) is **not** a cancel: work stays where it was and `recover` continues it.

## Structured answers

If the request has `expected_output` (a JSON Schema), the final answer must be JSON that matches it (a Markdown code
fence around it is accepted). Otherwise it is sent back once with the problem described.

## Result validators

Final answers pass through `ResultValidator`s. Two are always on (not empty; matches `expected_output`). Add your own
with `EngineBuilder::validator`.

## Using it

- From Rust: `zorix-runtime`'s `Engine`, which implements the `ExecutionEngine` trait of `zorix-core`.
- From a terminal: the [`zorix` command line](cli.md).
- From the [desktop app](desktop.md).

All three are thin faces over the same engine.
