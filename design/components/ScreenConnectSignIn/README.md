# Sign in to a service

Connecting GitHub, GitLab or Jira by signing in, beside pasting a token. Slack takes a pasted token (ADR-0053).

- Built from Modal (ConnectModal), Segmented, Field, Callout and Button.
- GitHub shows a device code; the others open the browser. The scopes asked for are named before the button.
- A provider that needs an app of your own says which redirect address to add.
