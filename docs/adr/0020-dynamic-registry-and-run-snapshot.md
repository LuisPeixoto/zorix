# ADR-0020: A dynamic tool registry, and a tool snapshot per run

Status: accepted (Phase 4)

## Context

MCP servers start, stop and change their tools while ZORIX runs, but an execution must be able to resume after a crash with the
same tools it started with, and the provider's prompt cache depends on a stable tool list (ADR-0015).

## Decision

- `ToolRegistry` gains `unregister`, `replace` and a change counter. `ToolSpec` gains `source` (`builtin`, `memory`, `mcp{server}`,
  `integration{connection}`, `sandbox`), shown in events and the UI.
- When an execution starts, the engine **snapshots the definitions it may use** (those its grants allow) and stores the snapshot with
  the execution. Every step and every resume offers exactly that list and runs the tool through the registry **only if its pinned
  definition hash still matches**; a mismatch fails the call with "the tool changed since this run started", never silently runs
  the new definition.
- A tool that disappears mid-run fails its call with a clear error. A tool added mid-run is not offered to runs already going.
- `Tool::describe_call(args)` returns a short, safe, human summary for approval cards.

## Consequences

- Slightly more storage per execution (a list of definitions). Stable prompts, honest resumes, no rug pull inside a run.
