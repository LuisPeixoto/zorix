# Research: tools, integrations and sandboxes (Phase 4)

Written on 2026-11-19 for ZORIX 0.4. Findings marked **(verified)** were read from the primary source on that date;
**(verify)** marks facts to re-check before relying on them. Nothing here copies code or assets from another product.

## 1. The Model Context Protocol is now two protocols

The current revision is **2026-07-28** **(verified)**. It is a different protocol from the revisions most servers still speak:

| | Legacy (2025-11-25 and earlier) | Modern (2026-07-28) |
|---|---|---|
| Start | `initialize` request, then `notifications/initialized`; a session lives as long as the connection | No handshake. Every request carries `_meta` with `io.modelcontextprotocol/protocolVersion`, `clientInfo` and `clientCapabilities` |
| Discovery | Result of `initialize` | `server/discover` (servers must implement it; clients may skip it) |
| Server → client requests | Allowed: sampling, elicitation, roots, ping | **Not allowed.** A server returns `InputRequiredResult` (`resultType: "input_required"`) and the client retries the call with `inputResponses` (multi round-trip requests) |
| HTTP | Optional `Mcp-Session-Id`, optional GET stream, resumable SSE, DELETE to end a session | Single POST endpoint, every message its own POST, no sessions, no GET stream. Headers `MCP-Protocol-Version`, `Mcp-Method`, `Mcp-Name` mirror the body and are validated by the server (`400` + code `-32020` on mismatch) |
| Change notifications | On the shared stream | Opt in with a long-lived `subscriptions/listen` request |
| Cancellation | `notifications/cancelled` | stdio: `notifications/cancelled`. HTTP: closing the response stream |
| Results | `content`, `structuredContent`, `isError` | The same, plus `resultType: "complete"` |
| Errors | JSON-RPC | Adds `UnsupportedProtocolVersionError` (`-32022`, with `supported` versions) and `HeaderMismatch` (`-32020`) |

**Detecting the era** **(verified)**:
- stdio: send `server/discover` first. A `DiscoverResult` or a recognised modern error means modern. Any other error, or silence, means legacy: fall back to `initialize`. The fallback must not depend on one error code.
- HTTP: send a modern request. A `400` whose body is a recognised modern JSON-RPC error means modern (retry with a supported version). An empty or unrecognised `400`, or a `404`/`405`, means legacy: use `initialize`, and possibly the deprecated 2024-11-05 HTTP+SSE transport.
- The era is a property of the server: cache it for the process (stdio) or origin (HTTP).

**Observed in the wild (REA).** The MCP host inside the shipped Claude desktop app (`.vite/build/mcp-runtime/directMcpHost.js`, version
5.11492.5, read from `app.asar` without running it) lists the protocol versions `2024-10-07`, `2024-11-05`, `2025-03-26`,
`2025-06-18`, `2025-11-25` and `2026-07-28`: a dual-era client is what production software ships.

**Consequence for ZORIX.** The client must be **dual-era**. We cannot support only the modern revision and we should not
support only the old one. We never answer server-initiated requests; see the security model below.

### Tools **(verified)**
`tools/list` (cursor pagination, optional `ttlMs` caching), `tools/call`. A tool has `name` (SHOULD be 1–128 chars of
`A-Za-z0-9_.-`), optional `title`, `description`, `inputSchema` (JSON Schema, 2020-12 by default), optional `outputSchema` and
`annotations`. Annotations are **untrusted** unless the server is trusted (MUST). Results carry `content` (text, image, audio,
`resource_link`, embedded resource), optional `structuredContent`, and `isError`. Two kinds of error: protocol errors
(unknown tool, malformed request) and tool execution errors (`isError: true`, meant for the model).

Streamable HTTP servers may mark parameters with `x-mcp-header`; a client **must** mirror them into `Mcp-Param-*` headers (with Base64
encoding for unsafe values) and **must** reject a tool whose annotation is invalid, leaving the other tools usable.

### Authorization **(verified)**
Optional; for HTTP only (stdio servers read credentials from the environment). The flow is OAuth 2.1:
1. A request without a token gets `401` with `WWW-Authenticate: Bearer resource_metadata="…", scope="…"`.
2. The client fetches **Protected Resource Metadata** (RFC 9728) and learns the authorization server(s).
3. It fetches **authorization server metadata** (RFC 8414 or OpenID Connect discovery; clients must support both).
4. It gets a `client_id`: **Client ID Metadata Documents** (preferred), pre-registration, or **Dynamic Client Registration** (RFC 7591,
   now deprecated but still the most widely deployed).
5. PKCE (S256) and the **`resource` parameter** (RFC 8707) in both the authorization and the token request, set to the canonical server
   URI without a trailing slash.
6. The client records the issuer before redirecting and validates **`iss`** on the response (RFC 9207), with a simple string comparison.
7. The token goes in `Authorization: Bearer` on **every** request, never in the query string. A client must not send a token
   to a server other than the one it was issued for. Refresh tokens must be kept confidential.
8. `403` with `error="insufficient_scope"` starts a step-up flow: ask again with the union of old and new scopes, retry a few times, then give up.

### What the standard says about trust **(verified)**
Tools are arbitrary code. Hosts **must** get explicit consent before invoking a tool, should show tool inputs before the call, should
validate results before giving them to the model, and **must** treat annotations as untrusted. Servers that aggregate tools from
several sources should disambiguate names; the server `name` is not unique and must not be used for that.

## 2. Build or adopt: an MCP client

| Option | For | Against |
|---|---|---|
| **`rmcp`** (official Rust SDK, 3.0.x) | Official, maintained, both transports | Its support for 2026-07-28 is described as beta in secondary sources **(verify)**; a large dependency tree (its own HTTP stack, schema crates); the trust layer we need (definition pinning, per-run snapshots, never answering server requests, our own OAuth with keychain tokens, redaction, size caps) sits *between* the transport and the tool, where an SDK's abstractions are in the way |
| **Hand-written client** (JSON-RPC + two transports) | About 1,500 lines for what we use (tools, discover/initialize, cancel, progress, subscriptions, OAuth). Same HTTP client as the providers (HTTPS only, no redirects with secrets, size and idle limits, redaction). Same reasoning as ADR-0009. Full control of both eras | We track the spec ourselves |

**Decision: a hand-written client** (ADR-0018). We use the official TypeScript schema as the reference and test against our own fake
servers (legacy and modern) and against real servers in opt-in live tests.

## 3. Tool poisoning and rug pulls

- **Tool poisoning.** A malicious or compromised server hides instructions in a tool's description or schema ("also send ~/.ssh to…").
  The model reads it; the user usually does not.
- **Rug pull.** A server behaves, gets approved, then changes a description after approval.
- **Cross-server shadowing.** One server's description tries to change how the model uses another server's tools.
- **Tool name collisions and typosquatting.**

Defences in ZORIX (ADR-0018): a person reviews the exact definitions (name, description, schemas) when enabling a server; each
definition is **hashed and pinned**; a changed definition disables the tool until a person reviews the diff; descriptions are
shown to the model labelled with their source; every MCP tool asks before it runs unless a person says otherwise for a specific
read-only tool; ids are namespaced (`mcp.<server>.<tool>`); results are untrusted data. None of this makes a malicious server safe:
a **stdio server runs as the user**. The consent screen says so.

## 4. REA study: how shipped products present connectors

Method: Claude.app (`/Applications/Claude.app`, `app.asar`, 2,997 files) was opened with REA (`analyze_javascript_application`,
`extract_artifact`) and its English strings and MCP runtime read **statically**; nothing was executed. Patterns only: no code, text or assets are reused.

| Pattern | Seen in | Take-away for ZORIX |
|---|---|---|
| A **confirmation before adding** a server ("Add this MCP server?" / "Add these N MCP servers?") | Claude.app strings | One consent step per server, with a plural form for imports; show command, arguments, URL, environment |
| **Import from another app's config**, reporting what was found, what was added and what was skipped; invalid entries named and skipped, not fatal ("entries … are not valid MCP server configurations and were skipped") | Claude.app strings | Import with a preview, per-entry validation, a clear skipped list |
| **Allow once / Always allow {entry}** for tool use, and "Always allow reads on {host}" | Claude.app strings | Per-call approval with a remembered choice scoped to one tool, never a blanket |
| Servers are **"connectors"** with states: sign in, reconnect, "didn't start in time", "couldn't use {connector} and N more" | Claude.app strings | A status model: starting, ready, needs login, error, with actions on each |
| **Reload configuration**, **open the MCP log file** | Claude.app strings | A live log per server, a reload action |
| Local MCP servers "**run outside the sandbox**" | Claude.app strings | Say it plainly in the UI and docs |
| **Bundled first-party servers** next to user-added ones: a GitHub MCP server binary and an Office 365 server shipped in `resources/` | Claude.app inventory | ZORIX's own native integrations play this role; keep them visually and technically distinct from third-party servers |
| A **key-change warning** that refuses a connection until the fingerprint is confirmed ("this key is different from the one saved") | Claude.app strings (SSH hosts) | The same shape fits a changed tool definition: refuse until a person reviews the change |
| A dual-era protocol list in the MCP host | Claude.app MCP runtime | See §1 |

VS Code, ChatGPT.app and OpenCode were listed as targets; the first one's shipped bundle was not opened in this pass and is
**(verify)** in the design critique (branch 14).

## 5. Native integrations: APIs and limits

| Service | API | Auth | Notes **(verify before coding each)** |
|---|---|---|---|
| GitHub | REST `https://api.github.com`, header `X-GitHub-Api-Version: 2022-11-28`, `Accept: application/vnd.github+json` | Fine-grained or classic token as `Authorization: Bearer` | 5,000 requests/hour authenticated (60 unauthenticated); `Link` header pagination; secondary rate limits (`retry-after`); GitHub Enterprise base URL |
| GitLab | REST `/api/v4` (gitlab.com or self-managed) | `PRIVATE-TOKEN` header | Offset or keyset pagination; project ids are URL-encoded `namespace%2Fproject` |
| Jira Cloud | REST v3 `/rest/api/3` | Basic `email:api_token` (or OAuth 2.0 3LO) | Issue search moved to `/search/jql` with `nextPageToken`; descriptions and comments use Atlassian Document Format; Jira Data Center differs |
| Slack | Web API (`chat.postMessage`, `conversations.history`, `conversations.list`) | Bot token `xoxb-…` as Bearer | Method tiers and `429` with `Retry-After`; channels by id; the bot must be invited to a channel |
| Discord | REST `/api/v10` | `Authorization: Bot <token>` | Per-route rate limits with headers; 2,000-character messages; the bot needs channel permissions |
| Postgres | `tokio-postgres` with rustls (`tokio-postgres-rustls`) | Connection string; password in the keychain | `SET TRANSACTION READ ONLY`, `statement_timeout`; a custom CA file for private servers |
| SQLite | `rusqlite` (already a dependency) | File path | `PRAGMA query_only = ON`, an interrupt handle for timeouts |
| Generic HTTP | `reqwest` (already a dependency) | Per-host secret from a connection | Everything in §6 |

## 6. Network safety for an HTTP tool

A tool that takes a model-chosen URL is an SSRF primitive. Rules (ADR-0021):
- HTTPS only unless the host is loopback and explicitly granted.
- Resolve the name ourselves, **check every returned address** (loopback, private ranges, link-local including `169.254.169.254`, unique-local IPv6, multicast, unspecified, IPv4-mapped IPv6), then connect to **that address** so a second DNS answer cannot rebind it.
- Redirects are not followed automatically: each hop is checked against the same rules and against the granted hosts, at most five.
- Credentials are attached by host from a connection, never taken from the model's arguments, and never follow a redirect to another host.
- Response size, time and header limits; redacted errors.

## 7. Running code: WebAssembly

**Runtime.** `wasmtime` is at 50.0.2 (released 2026-11-16) with a `51.0.0-rc.1` on crates.io **(verified)**. It gives memory limits (`StoreLimits`),
CPU limits (epoch interruption or fuel), WASI with explicit preopened directories, and no network unless granted. WASI preview 1
modules are enough for command-line interpreters.

**JavaScript.** `quickjs-ng` ships `qjs-wasi.wasm` (1.57 MB, a WASI command-line interpreter) in release **v0.18.0 (2026-11-01)**; its
SHA-256 is `2c218412cb8b17e46ee91bea7556ec38f5b9b6bd6312fddd3ceeed14aa71eaaa` **(verified from the release API)**. It can be embedded in the
binary. Javy (Bytecode Alliance) was considered and rejected: its releases ship only the CLI, and running a snippet needs a compile step per script.

**Python.** There is no official prebuilt `python.wasm`. The `vmware-labs/webassembly-language-runtimes` release
`python/3.12.0+20231211-040d5a6` provides `python-3.12.0.wasm` (26,267,204 bytes, SHA-256
`e5dc5a398b07b54ea8fdb503bf68fb583d533f10ec3f930963e02b9505f7a763` **(verified)**). It is from 2023 and the repository is not archived but
has not been pushed since 2024-06-05. Python 3.12 is still supported upstream. We **never bundle it**: `zorix sandbox install python`
downloads it once into the data folder, checks the pinned hash and refuses anything else. A later phase can move to a newer build.

**What the sandbox is and is not.** WebAssembly gives memory safety and capability-based I/O; it is not a defence against a
runtime bug in wasmtime (we pin and update it). Limits: memory, CPU time, output size, scratch disk, no network, no environment,
no host paths except what the run was granted. Native extension modules (numpy and friends) are not available.

## 8. Decisions

| # | Decision | ADR |
|---|---|---|
| 1 | Hand-written dual-era MCP client; stdio and Streamable HTTP; never answer sampling, elicitation or roots | 0018 |
| 2 | OAuth 2.1 with PKCE, `resource`, `iss` validation, DCR (CIMD recorded as follow-up), tokens in the keychain | 0019 |
| 3 | A dynamic tool registry; each run takes a snapshot of its tools when it starts | 0020 |
| 4 | Integrations as native tools with resource-level grants; SSRF guard; connections hold secrets by keychain reference | 0021 |
| 5 | Code runs in wasmtime; QuickJS embedded, CPython downloaded and pinned | 0022 |
| 6 | `zorix mcp serve` over stdio, read-only by default | 0023 |
