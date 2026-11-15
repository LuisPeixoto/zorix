# ADR-0005: An ordered, persisted event stream is the public face of an execution

Status: accepted (Phase 1)

## Context

Interfaces and later modules must follow executions live, and a restarted interface must catch up.

## Decision

Every state change appends an `ExecutionEvent { seq, execution_id, at, schema_version, kind, payload }` in the same
transaction as the change. After commit the event is broadcast on an in-process `EventBus` (tokio broadcast).
Subscribers can `replay(from_seq)` from the repository and then follow live; the bus handles lag by telling the
subscriber to replay. Kinds are dotted names (`execution.tool.requested`, …). Payloads never contain secrets.

## Consequences

- The UI, the CLI `events --follow` and future modules (workflows, metrics) use one mechanism.
- Event payloads are versioned contracts; adding fields is compatible, changing meaning needs a new `schema_version`.
