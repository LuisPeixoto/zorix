# ADR-0004: SQLite as the first repository, behind a trait

Status: accepted (Phase 1)

## Context

Desktop app, three operating systems, no server, a need for transactions (state change + event + checkpoint together),
and for recovery after a crash.

## Decision

`zorix-store` implements `ExecutionRepository` with SQLite (`rusqlite`, bundled so no system library is required), WAL
mode, versioned migrations and one transaction per state change. Tables: `executions`, `events` (monotonic `seq`),
`checkpoints`, `tool_calls`, `approvals`, `idempotency_keys`, `leases`. An in-memory repository in `zorix-runtime` backs
unit tests and shares a contract test suite with SQLite.

## Consequences

- One file, no daemon, easy backup; secrets are never stored in it (credentials belong to a later secret store).
- Postgres or a hosted backend implement the same trait later; the shared contract tests prove equivalence.
