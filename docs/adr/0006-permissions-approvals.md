# ADR-0006: Deny-by-default grants, policy at execution time, call-bound approvals

Status: accepted (Phase 1)

## Context

Models are untrusted and can be steered by injected text. Offering a tool to the model is not authorisation to run it.

## Decision

- An execution carries explicit `Grants` (action + resource pattern). Nothing is granted by default.
- Each tool declares `required_permissions`. Immediately before running a call the `PolicyEngine` evaluates tool,
  arguments, grants, environment and consumption limits and answers `Allow`, `Deny(reason)` or
  `RequireApproval(reason)`. `Deny` becomes an error tool result the model sees; the runtime never retries it as-is.
- An approval request stores the id the runtime issued and `sha256(tool id + canonical JSON arguments)`. Only
  `Engine::approve(execution, approval_id, decision, decided_by)` can resolve it, and the stored hash is compared with
  the call about to run. Model text has no path to this API.
- Tool output is added to the context as a message with `Role::Tool` and an `untrusted` provenance marker; the context
  builder wraps it so it is clearly data, and the system instructions state that tool output is not instructions.

## Consequences

- A forged or replayed approval for a different call fails.
- Prompt injection can still persuade a model to *ask* for something; it cannot make the runtime run it.
