# ADR-0019: OAuth for remote MCP servers, tokens in the keychain

Status: accepted (Phase 4)

## Context

Remote MCP servers (GitHub, Linear, Notion, Atlassian…) use OAuth 2.1. A desktop app is a public client with a loopback redirect.

## Decision

- Discovery: Protected Resource Metadata from `WWW-Authenticate` (or the well-known URL), then authorization server metadata via
  RFC 8414 **and** OpenID Connect discovery, in the priority order the spec gives.
- Registration, in order: a `client_id` the person supplied; **Dynamic Client Registration**; Client ID Metadata Documents are
  recorded as a follow-up because they need a hosted HTTPS document and the app has none yet.
- Authorization code with **PKCE S256**, the **`resource`** parameter in both requests (canonical URI, no trailing slash), a random
  `state`, and **`iss`** validation (RFC 9207, plain string comparison) before the code is used. The expected issuer is recorded before the browser opens.
- Redirect to `http://127.0.0.1:<random port>/callback` with a one-shot listener that accepts one request, checks `state`, shows a
  plain page and closes. The system browser is opened with the authorization URL; the URL is also printed for headless use.
- **Tokens** (access, refresh, expiry, scopes, client registration) are stored in the keychain under the server's id, as JSON, never in
  the database, events or logs; the in-memory type prints as `[redacted]`. A token is sent only to the server it was issued for
  (`resource`), only over HTTPS (or loopback), in the `Authorization` header, never in a URL.
- Refresh a little before expiry and on `401`; on `403 insufficient_scope` run step-up with the union of scopes, at most twice per call.
  A revoked or unrefreshable token puts the server in "needs login" and fails the call with that message.
- A server that does not support OAuth can use a static **bearer token** (or extra headers) from the keychain.

## Consequences

- Servers that only support CIMD or a pre-registered client need `--client-id`.
- Logout deletes the keychain entry and, when the server offers revocation, revokes the token.
