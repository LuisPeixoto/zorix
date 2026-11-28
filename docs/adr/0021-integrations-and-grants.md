# ADR-0021: Native integrations with resource-level grants

Status: accepted (Phase 4)

## Context

HTTP, Git hosts, ticket systems, chat and databases are what people want to connect first. They are also where a prompt
injection becomes damage: a message sent, an issue closed, a table dropped, data posted to an attacker.

## Decision

- **Connections** hold a name, a kind, non-secret settings and a keychain reference for the secret. A run is granted tools *and*
  resources: `network http:<host>`, `read|write github:<owner>/<repo>`, `read|write jira:<PROJECT>`, `read|write slack:<channel>`,
  `read|write db:<connection>`, `execute code:<language>`. Wildcards follow the existing rule (`*`, `prefix*`).
- **Writes ask.** HTTP methods other than GET and HEAD, every GitHub, GitLab, Jira write, every chat message and every database
  `execute` require approval; the card shows the target and a preview (body, diff, highlighted SQL). Reads do not ask when granted.
- **Secrets are attached by the tool**, chosen by host or connection, never read from model arguments, and never follow a redirect to another host.
- **HTTP is two tools** because a tool's approval is fixed: `http.get` (GET only, no approval, grants required) and `http.request`
  (any method, always asks). `webhook.send` posts a signed event to an address fixed by a `webhook` connection, never one the model
  chose, signed with HMAC-SHA256 over `<timestamp>.<body>` (`X-Zorix-Signature: sha256=…`, `X-Zorix-Timestamp`, `X-Zorix-Event`).
- **Model-supplied headers** cannot carry credentials (`Authorization`, `Cookie`, anything with auth, token, key, secret or session in
  its name) or change the connection; secrets come from a connection of kind `http` matched by host, over HTTPS (or to this computer).
- **No proxy.** These tools connect directly: a proxy resolves names itself and would defeat the address checks. A network that
  requires a proxy cannot use them in 0.4.
- **SSRF guard** for the HTTP tools, whose address a model chooses: HTTPS only (loopback only when granted); resolve, check every address against
  loopback, private, link-local, unique-local, multicast, unspecified and IPv4-mapped ranges, connect to the checked address; redirects
  are followed manually, at most five, each re-checked.
- **The address of a connection** (a Git host, Jira, Slack, Discord) is a different case, and this ADR first said it got the same guard,
  which it never did (corrected after the Phase 12 audit). A person chose it, so it is trusted more: HTTPS (plain HTTP only to this
  computer), never a redirect, the system proxy applies, and private networks and this computer are allowed, because a company's own
  GitHub Enterprise or Jira lives there. What it refuses is an address where no service lives: link-local (cloud metadata), the
  unspecified address, multicast, broadcast and reserved ranges, as an address in the connection and as what a name resolves to
  (`zorix_net::never_a_service`). A failed local lookup is not a refusal, so a network whose names only its proxy can resolve still works.
- **Databases**: `db.query` runs inside a read-only transaction (Postgres `SET TRANSACTION READ ONLY`; SQLite opens the file read-only, refuses any statement that is not read-only and installs an authorizer) with a statement timeout and
  row and byte caps; `db.execute` is a separate tool.
- Outbound **webhooks** are `webhook.send` with an HMAC-SHA256 signature header over the body; inbound webhooks belong to Phase 7.
- Limits: HTTP 30 s, 2 MB, 5 redirects; databases 1,000 rows, 5 MB, 15 s; page sizes per API.

## Consequences

- A granted `write` on one repository cannot touch another. A granted host cannot be used to reach a private network.
- Each integration is tested against a mock server that checks exact requests.
