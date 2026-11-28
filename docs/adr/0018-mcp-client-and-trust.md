# ADR-0018: A dual-era MCP client with a trust layer of our own

Status: accepted (Phase 4)

## Context

MCP revision 2026-07-28 removed the `initialize` handshake, sessions and server-initiated requests; revisions up to 2025-11-25 have
them. Most servers will speak the old shape for a while. A server is arbitrary code or an arbitrary remote service, and its tool
descriptions are read by the model ([research](../research/integrations-landscape.md)).

## Amendment (Phase 12): what an approval is of, and what may skip asking

The audit found that an approval took whatever definition was current at the click, so a server that changed a tool between the
person reading it and pressing *Approve* got the new text approved unseen; and that "don't ask" was allowed for a read-only tool that
said nothing about `openWorldHint`, which the specification defaults to true. Now an approval from a screen carries the hash of the
definition that was shown and is refused when the server lists another one (`PinAction::Approve { hash }`; `--approve` and
`--approve-all` on the command line stay "what is listed now", as the flags say), and a tool may skip asking only when it reports
`readOnlyHint: true` and `openWorldHint: false`; an earlier choice that the rule no longer allows is withdrawn when the tools are next read.

## Decision

- **Hand-written client**, not `rmcp`: JSON-RPC 2.0, **stdio** and **Streamable HTTP**, on the HTTP client the providers use
  (HTTPS only, size and idle limits, redacted errors). The trust layer sits between the transport and the `Tool` adapter, and an SDK would be in the way.
- **Dual-era.** stdio: probe `server/discover`, fall back to `initialize` on any non-modern answer. HTTP: try a modern request,
  inspect a `400` body, fall back to `initialize`. The **deprecated 2024-11-05 HTTP+SSE transport is not supported** in 0.4
  (it is deprecated since 2025-03-26 and servers that only speak it are rare). The era is cached per process or origin and
  re-probed when it fails.
- **Changes to tools.** A modern server announces them only to a client that opened `subscriptions/listen`, so the client opens one
  (and waits for its acknowledgement before it is returned, so no change is missed between listing and listening) and opens it
  again, telling listeners to look again, if it ends. Legacy stdio servers announce unasked. A legacy HTTP server's changes are
  seen at the next start or on Refresh, because ZORIX does not open the legacy GET stream.
- **`x-mcp-header`.** Parameters a server wants mirrored into `Mcp-Param-*` headers are mirrored (with the Base64 sentinel for
  unsafe values), and a tool whose annotation is invalid is left out with the reason.
- **Messages over SSE** are limited three ways (the parser's line limit alone was not a limit, found by the Phase 12 audit): a line at most 1 MiB, the data of one event at most 4 MiB however many lines it is spread over, and 16 MiB for a whole streamed answer; a JSON body or a stdio line may be 4 MB.
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
