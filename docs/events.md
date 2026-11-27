# Events

Everything that happens to an execution is an **event**: ordered, numbered, stored, and published. Interfaces, the CLI
and later modules follow executions through events, with one mechanism ([ADR-0005](adr/0005-events.md)).

## Shape

```json
{ "seq": 14, "execution_id": "01a1e0db-…", "at": 1794775417686, "schema_version": 1,
  "kind": "execution.tool.completed", "payload": { "call_id": "call_2_0", "tool": "notes.put", "duration_ms": 1, "output_bytes": 18 } }
```

- `seq` is global and strictly increasing, so a subscriber can resume after the last one it saw.
- `at` is milliseconds since the Unix epoch (UTC).
- `schema_version` is `1`. Adding fields keeps it; changing what a field means will raise it.
- Payloads never contain secrets (see [permissions](permissions.md#credentials)).

## Kinds

| Kind | When | Payload (main fields) |
|---|---|---|
| `execution.created` | Stored | `model`, `prompt_chars`, `conversation`, `parent`, `grants` |
| `execution.queued` | Waiting for a worker | |
| `execution.started` / `execution.resumed` | A worker took it | |
| `execution.progress` | After each model answer that asks for tools | `iteration`, `tool_calls_requested`, `active_ms` |
| `execution.model.requested` | A model call starts | `iteration`, `model`, `messages`, `tools`, `estimated_input_tokens`, `estimated_tool_tokens`, `token_factor`, `history_edited`, `segments` (each context segment: `origin`, `tokens`, `kept`) |
| `execution.model.responded` | The model answered | `iteration`, `model`, `finish`, `text_chars`, `tool_calls`, `usage` |
| `execution.context.summarised` | Turns that no longer fit were summarised ([ADR-0043](adr/0043-summaries-of-long-conversations.md)) | `folded_messages`, `covers`, `summary_chars`, `input_tokens`, `output_tokens`, `cost_micros` (never the text) |
| `execution.model.retrying` | A temporary model error | `attempt`, `delay_ms`, `error` |
| `execution.tool.requested` | The model asked for a tool | `call_id`, `tool`, `arguments` (redacted), `call_hash`, `source` (`builtin`, `memory`, `mcp:<server>`, `integration:<connection>`, `sandbox`) |
| `execution.tool.denied` | Refused (policy, person, or unknown tool) | `call_id`, `tool`, `reason`, `by` |
| `execution.tool.unattended` | A call ran without asking because of the worker's level (ADR-0039) | `call_id`, `tool`, `arguments`, `call_hash`, `level`, `reason`, `summary` |
| `execution.tool.suggested` | A worker at the Suggest level proposed a call instead of making it | `call_id`, `tool`, `arguments`, `reason`, `summary`, `by` |
| `execution.tool.started` | A tool run begins | `call_id`, `tool`, `attempt` |
| `execution.tool.completed` | It returned | `call_id`, `tool`, `duration_ms`, `output_bytes` |
| `execution.tool.failed` | Invalid arguments or the tool failed | `call_id`, `tool`, `error` |
| `execution.waiting_approval` | Suspended for a person | `approval_id`, `call_id`, `tool`, `arguments`, `call_hash`, `reason`, `person_only` (true: only a person may answer, as for `memory.remember`), `source`, `definition_hash` (the pinned definition of the tool), `summary` (a sentence about this call, from the tool), `outcome_unknown: true` after a crash |
| `execution.waiting_children` | Suspended for the agents it asked | `call_id`, `children` (ids), `tasks` |
| `execution.child.started` | A sub-run was created and started (one per task) | `child`, `agent`, `depth`, `task` (first 200 characters) |
| `execution.child.settled` | A sub-run finished and its result was handed back | `child`, `agent`, `state`, `steps`, `tool_calls`, `cost_micros` |
| `execution.approval.granted` / `.denied` | A person answered | `approval_id`, `tool`, `decided_by`, `note` |
| `execution.result.rejected` | A validator rejected the answer | `validator`, `reason`, `attempt` |
| `execution.checkpoint` | A durability point before suspending | `step` |
| `execution.paused` | Stopped at a step boundary | `step` |
| `execution.recovered` | Put back in the queue after a restart | |
| `execution.completed` | Finished with a result | `result_chars`, `iterations`, `tool_calls`, `usage` |
| `execution.failed` | Finished with an error | `kind`, `message`, `limit`, `usage` |
| `execution.cancelled` | Stopped on request | `reason` |

Live text (the words of an answer while a model is still writing it) is **not an event**: it is never stored and has no sequence number. It reaches a
screen through `ExecutionEngine::live()`, pieces may be dropped when a reader is slow, and the stored `execution.model.responded` is the record that replaces
whatever was drawn. Runs made by an agent record it in the execution (`agent`, `version`, `hash`) and in `execution.created`; evaluation runs carry the metadata
label `zorix.eval`.

## Following an execution

```rust
let mut events = engine.subscribe(&tenant, id, 0).await?;   // stored events, then live ones
while let Some(event) = events.next().await { /* … */ }     // ends after completed, failed or cancelled
```

The stream replays what is stored after `after_seq`, then follows live, **without gaps or repeats**, even if the
subscriber falls behind: the in-process bus only wakes subscribers up, and what they emit always comes from the store.

From the terminal: `zorix events <id> --follow`. In the desktop app, the timeline is this stream.

## Guarantees

- An event is written **in the same transaction** as the change it describes. A crash cannot leave a state change without
  its event.
- Events of one execution are in the order the changes were committed.
- Publishing happens after the commit and never fails the execution.
