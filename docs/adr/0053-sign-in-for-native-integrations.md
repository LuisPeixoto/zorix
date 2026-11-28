# ADR-0053: Signing in to the native integrations

Status: accepted (Phase 12). Amends [ADR-0021](0021-integrations-and-grants.md), where a connection only took a token.

## Context

Personal tokens are broad, long-lived and copied by hand. GitHub, GitLab and Jira offer OAuth, which gives tokens that can be
revoked and, for GitLab and Jira, that expire and renew.

## Decision

- A GitHub, GitLab or Jira connection can be **signed in to** instead of given a token. Pasting a token keeps working.
- **GitHub:** the device flow. A code is shown and the person enters it on the page; ZORIX never sees a password.
- **GitLab:** authorization code with PKCE, answered on `http://127.0.0.1:53682/callback`. The port is fixed because these
  services accept only the address registered with the app.
- **Jira:** Atlassian's three-legged flow with your own app's id and secret, then the site is matched among those the token
  reaches and the API is called at Atlassian's gateway for it.
- **Apps.** ZORIX's own app ids are compiled in from `ZORIX_GITHUB_CLIENT_ID` and `ZORIX_GITLAB_CLIENT_ID` when set; every
  provider also takes your own id. A build with neither says so.
- **Slack is not signed in to.** Slack requires an HTTPS redirect (a `localhost` one only with PKCE, which can limit bot
  scopes and cannot be turned off), and the app a person makes already shows its bot token, so signing in would add nothing.
- A signed-in connection is marked `auth: "oauth"` in its settings; tokens and the renewal data are in the keychain
  (`zorix-oauth`), renewed one at a time, and forgotten on removal (revoked first where the service allows it: GitLab).

## Security review

- **Where tokens go.** The addresses come from the connection (or are Atlassian's). Token calls obey the same network rules as
  MCP sign-in: HTTPS (plain HTTP only to this computer), no redirects, the resolved address checked and used.
- **The answer.** Only the callback carrying this sign-in's `state` is taken; PKCE binds the code for GitLab.
- **The code page.** GitHub's verification page must be on the host of the connection, or the sign-in stops before any code is
  shown. The window opens it by the connection's name; it cannot ask ZORIX to open an address of its own.
- **Claims.** Settings cannot say `oauth` without a sign-in, and nothing is added unless the sign-in succeeds.
- **Scopes** are the smallest the tools need: GitHub `repo` (classic apps have no narrower scope that writes issues), GitLab
  `api`, Jira `read:jira-work write:jira-work offline_access`. Grants still decide what a run may do.
- **Revocation** is possible only for GitLab without the app's secret; for GitHub and Jira, remove the app in your account's
  settings.
- Tried in `crates/zorix-integrations/tests/signin.rs`, `crates/zorix-hub/tests/hub.rs` and the CLI tests, against a mock
  service. **Not tried against the real services** (that needs registered apps).
