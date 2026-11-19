# ADR-0018: A dual-era MCP client with a trust layer of our own

Status: accepted (Phase 4)

## Context

MCP revision 2026-07-28 removed the `initialize` handshake, sessions and server-initiated requests; revisions up to 2025-11-25 have
them. Most servers will speak the old shape for a while. A server is arbitrary code or an arbitrary remote service, and its tool
descriptions are read by the model ([research](../research/integrations-landscape.md)).

## Decision

- **Hand-written client**, not `rmcp`: JSON-RPC 2.0, **stdio** and **Streamable HTTP**, on the HTTP client the providers use
  (HTTPS only, size and idle limits, redacted errors). The trust layer sits between the transport and the `Tool` adapter, and an SDK would be in the way.
- **Dual-era.** stdio: probe `server/discover`, fall back to `initialize` on any non-modern answer. HTTP: try a modern request,
  inspect a `400` body, fall back to `initialize`, and then to the deprecated HTTP+SSE transport only if the server answers like one.
  The era is cached per process or origin and re-probed when it fails.
- **Never answer a server's requests.** Legacy `sampling/createMessage`, `elicitation/create` and `roots/list`, and the modern
  `InputRequiredResult`, are refused: sampling would let a server spend model tokens, elicitation would let it ask a person for
  secrets outside ZORIX's consent screens. `ping` is answered. A call that needs input fails with a message that says so.
- **Consent.** Adding a server is a person's act. The screen shows the command, arguments, working directory and requested environment
  variable *names* (stdio), or the URL and OAuth state (HTTP), and says that a stdio server runs as the user without a sandbox.
- **Pinning.** When a person enables a server they review each tool's definition. The SHA-256 of the canonical JSON of
  `{name, description, inputSchema, outputSchema}` is stored. A different hash on a later `tools/list` **disables that tool** until a
  person reviews a diff. New tools arrive disabled. Annotations are stored as hints only.
- **Ids and grants.** A tool is `mcp.<server>.<tool>` (server names are ours and unique). A run only sees servers it was granted
  (`use_tool mcp.<server>.*` or one tool). Names that are not 1–128 characters of `A-Za-z0-9_.-` are rejected, as are tools whose
  `x-mcp-header` annotation is invalid.
- **Approval.** Every MCP tool asks first by default. A person may mark specific tools of a server "do not ask" **only** if the
  server reports `readOnlyHint` for them; `destructiveHint`, `openWorldHint` or missing annotations always ask.
- **Results are untrusted data**, wrapped like other tool results, with images and audio reduced to a placeholder, size-capped, and secret-redacted in events.
- **Processes.** Spawned without a shell, with a clean environment plus the variables the person approved, in their own process
  group; stderr goes to a 1 MB redacted ring log; stdin is closed first on shutdown, then terminate, then kill; a crash restarts
  with backoff, at most five times.
- **Limits:** 50 servers, 200 tools per server, 4 MB per message, 60 s per call, 30 s to start.

## Consequences

- We carry protocol upkeep. Mitigation: a protocol-test suite with a fake server in each era and opt-in live tests.
- A malicious stdio server can still do what the user can do. The product says so and asks first.
