# ADR-0034: The workflow host and triggers

Status: accepted (Phase 7)

## Context

Nothing in ZORIX runs in the background today. One process owns the data folder (`DataDirLock`), and the command line is short-lived.
A schedule or a webhook needs something that is awake. We must say where, and what happens when it is not.

## Decision

- A **`WorkflowHost`** (in `zorix-workflows`) owns the triggers: one timer task that sleeps until the earliest next fire or due wait,
  an event subscription, and an optional webhook listener. Only one process owns the data folder, so only one host exists.
- It runs in **the desktop app while it is open**, and under **`zorix serve`**, a headless command that takes the lock and runs the host
  until it is stopped. The workflows screen says so: "Schedules and webhooks run while ZORIX is open, or under `zorix serve`".
- **A fire missed while no host ran is skipped and recorded** ("missed while ZORIX was closed"), not replayed. Replaying a morning
  report at night, or ten of them, is worse than missing it. Timers (waits, time-outs) are durable and resume.
- **Schedules:** every N minutes (at least 5), at times on chosen weekdays, or a cron expression, each with a time zone that defaults
  to the system's. Next fires are computed with the time zone's rules, across daylight saving.
- **Events:** a run of an agent or team finished or failed (optionally of one agent), and *after another workflow* finished. Events
  arrive through the `EventPublisher` port, with a backfill pass at start for what was missed while down. A chain is cut at depth 4
  and a workflow never triggers itself.
- **Webhooks** are **off by default**. When on (Settings › General, or `zorix serve --bind`), the listener binds to loopback
  (`127.0.0.1:7788`) and serves `POST /hooks/<workflow>/<token>`. The token is a per-workflow secret kept in the keychain and compared in
  constant time; a wrong token or an unknown workflow gets the same 404. An optional shared secret checks a GitHub-style
  `X-Hub-Signature-256`. The body is capped at 1 MB, and a payload filter (a condition over the JSON) can ignore events. A disabled
  workflow answers 404. Reaching the listener from outside is the person's choice (a tunnel or a reverse proxy) and the guide says so.
- A payload is **data, never instructions**: it reaches a step only through a template, and agent prompts mark it as untrusted input.

## Consequences

- New dependencies: a cron parser, time zone data and a small HTTP server (`axum`).
- A laptop that sleeps misses its schedules; the answer for always-on work is `zorix serve`, not a hidden daemon.
- Exposing a port is explicit, token-protected and bounded; nothing listens until a person turns it on.
