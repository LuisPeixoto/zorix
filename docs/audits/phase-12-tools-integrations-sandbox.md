# Audit: tools, integrations and sandbox (Phase 12)

2026-11-28 · commit `1bd2e86` (v0.11.0) · audited by reading the code of Phase 4 against
[sections 2 to 14](../engineering-guidelines.md) of the guidelines, with the questions written in the
[template](README.md). Status: **open**, closed with the release of v0.12.0.

## Scope

Looked at:

- **The MCP client and server** (`zorix-mcp`, and `zorix mcp serve` in `zorix-cli`): both protocol eras, the stdio and HTTP
  transports, OAuth, tool pinning and review, the catalogue and the import of other clients' settings.
- **The native integrations** (`zorix-integrations`): the guarded HTTP tools, GitHub, GitLab, Jira, Slack, Discord, SQLite and
  Postgres, credentials and the network guard.
- **The built-in tools** (`zorix-tools`): the workspace file tools, their path rules and their output caps.
- **The sandbox** (`zorix-sandbox`): limits, mounts, the interpreters and their checksums, and the escape tests.
- **Where grants meet calls**: the permission patterns in `zorix-core`, and how workflow actions call tools
  (`zorix-workflows`).

Left out: agents, teams and orchestration (Phase 13), the rest of workflows, projects and the workforce (Phase 14), and the
interface (Phase 15). Where this audit touched them (the workflow launcher calls tools) it says so.

## Findings

A security finding is fixed first. Every finding is an issue in the milestone
[v0.12.0](https://github.com/LuisPeixoto/zorix/milestone/3). A row marked **†** names a suspicion: it is proved with a test at the
start of its branch and dropped, with a note here, if the test cannot show it.

| # | Section | Finding | Severity | Issue | Fixed in |
|---|---|---|---|---|---|
| F1 | 6 | A Jira search is widened to other projects by unbalanced parentheses in the JQL | High | [#52](https://github.com/LuisPeixoto/zorix/issues/52) | `fix/jira-scope` |
| F2 | 6 | A workflow HTTP step builds its grant from its own, templated, URL, so it reaches `localhost` and private hosts with no person involved | High | [#53](https://github.com/LuisPeixoto/zorix/issues/53) | `fix/workflow-action-checks` |
| F3 | 6 | Workflow actions skip argument validation and the policy (`forbidden`, open-world); a Discord channel id goes raw into the path | High | [#54](https://github.com/LuisPeixoto/zorix/issues/54) | `fix/workflow-action-checks` |
| F4 | 6 | A symlink in the workspace routes around a grant (`docs/l -> ../secret` passes `docs/*`) and around the reserved `.zorix` folder | High | [#55](https://github.com/LuisPeixoto/zorix/issues/55) | `fix/workspace-links` |
| F5 | 6 | MCP OAuth discovery follows the server's own `resource_metadata` address to any host, private and loopback included | High | [#56](https://github.com/LuisPeixoto/zorix/issues/56) | `fix/oauth-discovery-ssrf` |
| F6 | 6 | `http:` wildcard grants are text prefixes: `api.example.com*` matches `api.example.com.evil.net` | Medium | [#57](https://github.com/LuisPeixoto/zorix/issues/57) | `fix/grant-wildcards` |
| F7 | 6 | A write's temporary file follows a symlink already at that path, and `a.txt` and `a.md` share one temporary name | Medium | [#58](https://github.com/LuisPeixoto/zorix/issues/58) | `fix/workspace-links` |
| F8 | 6 | Native integration base URLs have no check for addresses where no service lives (link-local, multicast, reserved), and ADR-0021 says they have the same guard as the HTTP tools (they are a person's choice, HTTPS-only, so the risk is small: reclassified from Medium) | Low | [#59](https://github.com/LuisPeixoto/zorix/issues/59) | `fix/integration-base-urls` |
| F9 | 6 | Approving an MCP tool approves whatever definition is current at the click, not the one that was shown | Medium | [#60](https://github.com/LuisPeixoto/zorix/issues/60) | `fix/mcp-review` |
| F10 | 6 | A tool with no `openWorldHint` can be set to "don't ask", while everything else treats it as open-world | Medium | [#61](https://github.com/LuisPeixoto/zorix/issues/61) | `fix/mcp-review` |
| F11 | 6 | Any local request to the OAuth callback ends a sign-in, whatever its `state` | Low | [#62](https://github.com/LuisPeixoto/zorix/issues/62) | `fix/oauth-discovery-ssrf` |
| F12 | 6 | `zorix mcp serve`: `zorix.status` and `zorix.result` read any run of the tenant, not only the ones this server started | Medium | [#63](https://github.com/LuisPeixoto/zorix/issues/63) | `fix/mcp-serve-run-scope` |
| F13 | 7, 8 | A streamed MCP answer is not bounded: one event piles up `data:` lines without limit | Medium | [#64](https://github.com/LuisPeixoto/zorix/issues/64) | `fix/mcp-caps` |
| F14 | 7, 8 | MCP `structuredContent` passes uncapped, and the number of content items is unbounded | Medium | [#65](https://github.com/LuisPeixoto/zorix/issues/65) | `fix/mcp-caps` |
| F15 | 8 | A stopped stdio MCP server can leave child processes running (Windows has no Job Object; on Unix a leader that exits on its own leaves its group; no SIGTERM step) | Medium | [#66](https://github.com/LuisPeixoto/zorix/issues/66) | `fix/mcp-process-tree` |
| F16 | 6 | A stdio server with no working folder runs in ZORIX's own folder, not the data folder | Low | [#67](https://github.com/LuisPeixoto/zorix/issues/67) | `fix/mcp-process-tree` |
| F17 | 8 | An expired legacy HTTP session (404) is never re-initialised, so every call fails until a manual restart | Medium | [#68](https://github.com/LuisPeixoto/zorix/issues/68) | `fix/mcp-http-session` |
| F18 | 8 | `initialize` is cancelled on timeout (the specification forbids it), an HTTP cancel sends nothing, a string id is never matched, the cached era is never read (the documentation said it was; corrected rather than implemented) | Low | [#69](https://github.com/LuisPeixoto/zorix/issues/69) | `fix/mcp-http-session` |
| F19 | 8 | A stderr line over 16 KiB garbles the next one | Low | [#70](https://github.com/LuisPeixoto/zorix/issues/70) | `fix/mcp-caps` |
| F20 | 6, 8 | **†** No step-up on `insufficient_scope`; a reused registration keeps an old loopback port | Low | [#71](https://github.com/LuisPeixoto/zorix/issues/71) | `fix/mcp-oauth-registration` |
| F21 | 8 | MCP import can give two servers the same name; keychain names lowercase the variable, so `FOO` and `foo` collide | Low | [#72](https://github.com/LuisPeixoto/zorix/issues/72) | `fix/mcp-import-names` |
| F22 | 3 | Workflow Discord sends always fail (`channel`/`text` against `channel_id`/`content`), and an HTTP step with an explicit port always fails | Medium | [#73](https://github.com/LuisPeixoto/zorix/issues/73) | `fix/workflow-send-and-ports` |
| F23 | 6 | **†** Some Windows reserved names and characters are accepted (`COM0`, `LPT0`, `CONIN$`, `CONOUT$`, `<>"\|?*`); 8.3 short names may reach `.zorix` | Medium | [#74](https://github.com/LuisPeixoto/zorix/issues/74) | `fix/workspace-windows-names` |
| F24 | 6, 14 | Catalogue servers are not pinned (`npx -y`, `@latest`, `uvx`), so the code behind a pinned tool definition can change | Medium | [#75](https://github.com/LuisPeixoto/zorix/issues/75) | `feat/mcp-catalogue` |
| F25 | 8 | With `--allow-runs`, `mcp serve` holds the data-folder lock, so a run waiting for approval cannot be answered anywhere | Medium | [#76](https://github.com/LuisPeixoto/zorix/issues/76) | `feat/mcp-serve-runs-approvals` |
| F26 | 12 | A refused server request is dropped, so it never reaches a log or the screen | Low | [#77](https://github.com/LuisPeixoto/zorix/issues/77) | `feat/mcp-sampling-elicitation` |
| F27 | 7, 6 | Postgres loads `max_rows + 1` rows before its byte check and `SHOW`/`EXPLAIN` are uncapped; **†** `READ ONLY` against `dblink_exec` and `pg_terminate_backend`; **†** SQLite `VACUUM INTO` through `db.execute` | Low | [#78](https://github.com/LuisPeixoto/zorix/issues/78) | `fix/db-tool-limits` |
| F28 | 2, 6 | Sandbox escape tests are missing for symlinks, mounts and Windows paths, and the Windows WASI case of 0.9.x has no regression test | Medium | [#79](https://github.com/LuisPeixoto/zorix/issues/79) | `test/sandbox-escapes` |
| F29 | 9 | ADR-0008, 0018, 0021, 0022 and 0023 say what the code does not do | Low | [#80](https://github.com/LuisPeixoto/zorix/issues/80) | `docs/phase-12` |
| F30 | 11 | About 170 validation sentences of these areas are text, not codes | Low | [#26](https://github.com/LuisPeixoto/zorix/issues/26) | `refactor/reasons-tools-integrations` |

The audit keeps going while the fixes are written. A new row is added, with its issue, whenever a fix or a feature shows one.
Still to look at: MCP conformance against both eras beyond the rows above, the output cap of every tool, OAuth refresh under
concurrent calls, proxy and certificate variables for stdio servers, Slack channel paging, and hard links inside a workspace.

Kept as designed, and written down so nobody has to rediscover it:

- A stdio server's secrets are in its process environment. The program has to read them from somewhere, and the keychain
  is not readable by a child. They are never in the database, a log or an event.
- Tools that talk to the network (`http.*`) refuse private addresses unless a grant names that exact host, with no wildcard.
- A model-visible tool result is cut after the tool builds it and before it is stored, not before it is built. Each tool
  also applies its own cap first; the table is in [tools](../tools.md).

## Checked, nothing found

| Section | What | How |
|---|---|---|
| 6 | The HTTP tools resolve a host once, pin the client to the checked addresses and re-check every redirect hop | `zorix-integrations/src/net.rs`, `http_tools.rs` and their tests |
| 6 | IPv4-mapped, NAT64, 6to4, Teredo, ISATAP and unique-local IPv6 forms are refused as private | `net.rs` tests |
| 6 | MCP OAuth uses PKCE S256, a random `state`, the issuer check of RFC 9207, a loopback redirect with a Host check, and the `resource` parameter | `zorix-mcp/src/oauth.rs`, `tests/oauth.rs` |
| 6 | The SQLite tool opens read-only, refuses ATTACH and non-listed PRAGMAs, and the Postgres tool starts a read-only transaction before the statement | `zorix-integrations/tests/db_sqlite.rs`, `db_postgres.rs` |
| 6 | The sandbox sees no environment, no host file and no network | `zorix-sandbox/tests/javascript.rs`, `python.rs` |
| 6 | Interpreters are pinned by SHA-256, checked at download and at every load | `zorix-sandbox/src/runner.rs`, `install.rs` |
| 6 | Header names and values from the model are filtered (no auth-like names, no control characters) | `http_tools.rs` tests |
| 8 | A server that crashes or hangs does not stop the run | `zorix-mcp/tests/client.rs`, `manager.rs` |

## Not verified

- The full pass over the interface of these areas (Phase 15 audits every screen).
- Real accounts: GitHub, GitLab, Jira, Slack and Discord are checked against mock servers, and the live script uses public
  data without a token.
- Windows and Linux for the new work, until CI runs it.
