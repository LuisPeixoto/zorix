# ADR-0023: ZORIX as an MCP server, read-only by default

Status: accepted (Phase 4), amended by [ADR-0052](0052-runs-and-approvals-over-mcp.md)

## Context

People want Claude Code, Cursor or another MCP client to use ZORIX's memory and documents.

## Decision

- `zorix mcp serve` speaks MCP over **stdio** in both eras (the server side of ADR-0018's detection: a request with modern `_meta`
  is served statelessly; `initialize` selects legacy).
- Default tools: `memory.search`, `knowledge.search`, `history.search`, over the **reader** (no data-folder lock, so it works while
  the desktop app is open), scoped by `--tenant` and `--project`.
- `--allow-runs` adds `zorix.run`, `zorix.status` and `zorix.result` (the last two follow only runs the server started, unless `--allow-read-runs`, after the Phase 12 audit found they read any run of the tenant). These take the data-folder lock. The tool grants of the new
  execution come from flags (`--grant`); a run that needs approval answers "waiting for approval in ZORIX", and approval happens in
  ZORIX, never through the MCP client.
- Nothing writes memory through this server.
- stdout carries only protocol messages; logs go to stderr.

## Consequences

- Memory stays under the person's control: an outside agent can read, not remember.
