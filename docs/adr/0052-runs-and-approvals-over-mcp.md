# ADR-0052: Runs and approvals over MCP

Status: accepted (Phase 12). Amends [ADR-0023](0023-zorix-as-mcp-server.md).

## Context

With `zorix mcp serve --allow-runs`, the server holds the data folder's lock, so a run it started that stops for an approval
could not be answered anywhere until the server was closed. People also want to see what runs and what waits from the tool
they work in. An MCP client is a program, often steered by a model: its word is not the person's.

## Decision

- With `--allow-runs`, the server adds `zorix.runs`, `zorix.approvals` and `zorix.cancel`. A client sees the runs this server
  started (every run only with `--allow-read-runs`) and stops only the ones it started.
- `--allow-approvals` adds `zorix.answer_approval`. Before it runs, ZORIX asks the person through the client
  (`elicitation/create`, or an `input_required` result in the 2026-07-28 revision), showing the tool, the run, why it needs a
  person and the whole arguments. Only an explicit yes goes on.
- The answer names the call by id **and** hash, and the engine refuses a hash that is not the pending one.
- Resources: `zorix://agents/{name}` (never the instructions) and, with `--allow-runs`, `zorix://runs/{id}`.

## Security review

- **Who answers.** ZORIX cannot see whether a client really shows the question. A client that answers by itself would approve in
  the person's name, which is why the flag is off by default, is named for what it does, and the documentation says to turn it on
  only for a client you trust to ask. A client that does not offer elicitation is refused and nothing happens.
- **What is approved.** Nothing a person was not shown: a call whose arguments are longer than 4 000 characters can only be
  refused through MCP; hidden and reordering characters are written out, so arguments cannot disguise themselves.
- **What only ZORIX approves.** Tools marked as needing a person who sees them (memory writes, among others), and tools this
  process does not know, are never approved through MCP.
- **Replay.** A modern client's answer carries a state signed with a secret of the process, bound to the tool, the arguments and
  the question, and used once. At most 8 questions wait at a time, for 5 minutes each.
- **Reach.** Scope is checked again when the call runs, not only when listing. A run that is not the client's is refused with the
  same words as one that does not exist. Resource addresses are matched against names and ids, never read as paths.
- Tried in `crates/zorix-cli/tests/mcp_serve.rs` and `crates/zorix-mcp/tests/server.rs`: no flag, no elicitation, a no, another
  hash, a made-up or reused state, a person-only tool, another's run, addresses that are not resources.

## Consequences

- A run started from an editor can be finished from it, with the person still deciding.
- The decision is recorded as `mcp client, confirmed by the person`, so the audit trail says how it was given.
