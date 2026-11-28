# ADR-0051: What an MCP server may ask while its tool runs

Status: accepted (Phase 12). Amends [ADR-0018](0018-mcp-client-and-trust.md), which refused every request from a server.

## Context

Servers use **sampling** (ask the client's model) and **elicitation** (ask the person, or have them open a page) to finish
some tools. Refusing all of it made those tools fail. Answering it carelessly would let a server spend the person's tokens,
collect secrets, send them to a page, or draw words on screen that look like ZORIX's.

## Decision

- ZORIX declares `sampling` and `elicitation` (form and URL) to servers. They are answered only **during a tool call a run
  made**, in both revisions: server requests in the legacy ones, `input_required` and the repeated call in 2026-07-28.
- **Every request waits for the person.** No setting answers by itself. Sampling is *allow once* or *refuse*; a form is sent
  only when the person sends it; a page is shown with its domain and opened only on a click.
- **Nobody watching, nothing asked.** The desktop app (while open) and the terminal following a run with `--approve ask` can
  answer; `zorix serve`, `zorix mcp serve` and non-interactive runs refuse at once. A question without an answer for five
  minutes is refused.
- A model call uses the **run's model** (the server's model preferences are ignored), has no tools and nothing of the run's
  conversation, is capped at 4 096 output tokens and the run's remaining token budget, and its usage and cost are the run's.
- The person's time is not the tool's: it does not count against the tool's timeout or the run's working time.

## Security review

- **Bounded input.** At most 5 requests per tool call; sampling takes 32 messages and 32 KiB of text only (no images, tools or
  tool choice); a form takes 16 flat fields (text, number, integer, boolean, one of up to 50 choices); a URL must be `https`
  (or `http` on this computer), without a user name.
- **Bound answers.** A request has a hash of the server and exactly what it asks; an answer names it and counts once. A form's
  values are checked against the fields again in the backend. The page opened is the one in the waiting request, never a value
  the window sends.
- **Which run asks.** A legacy server does not say which call a request belongs to, so ZORIX answers only while exactly one
  call to that server runs, and refuses otherwise.
- **Untrusted text.** Everything the server wrote is shown as plain text; on the terminal, control and reordering characters are
  written out. The model is told the messages come from a tool server, they are marked untrusted, and the answer goes back to
  the server, not into the run.
- **Secrets.** ZORIX cannot know what a person types. The card and the terminal say never to type a password or key there.
- **Record.** Each request and what came of it is in the tool call's event (`server_requests`), and a refusal is in the server's
  log.
- Tried in `crates/zorix-runtime/tests/asks.rs`, `crates/zorix-mcp/tests/client.rs` and `http.rs`, and the parsers' own tests.

## Consequences

- Tools that need an answer work, with the person in the loop each time.
- A waiting request lives in memory: closing ZORIX ends it, and the interrupted call is handled like any other.
- **Not covered:** the cost of a model call is not estimated before it is allowed (the card shows the token cap).
