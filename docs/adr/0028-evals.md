# ADR-0028: Evaluations are deterministic first, with an optional judge

Status: accepted (Phase 5)

## Context

A person changing an agent needs to know whether it got better or worse, across versions and models, without trusting a
feeling.

## Decision

- An eval **case** is a prompt, optional input, an optional fixture workspace, tags and **checks**: contains, does not
  contain, regex, JSON Schema, a tool was (not) used, cost, steps and tool calls at most, final status. A case may add a **judge**
  (a rubric, a model, a pass score).
- An eval **run** is one agent version with one model; a report compares two runs case by case (pass, score, cost, steps and
  the deltas).
- Cases run through the AgentRunner with the narrowing rules: `write` grants are dropped unless the case allows writes,
  `--offline` also drops `network`, there is no conversation, and the run is **not** recorded as past work.
- **Approvals are refused automatically** (`evaluations refuse approvals`), except for tools the case lists and that are not
  person-only. At most two cases run at once.
- The judge is one call through the router. The rubric is the system message; the answer under test is wrapped as
  untrusted data. Its model and cost are recorded in the result and shown.

## Consequences

- Evals are repeatable and free by default; the judge is an explicit, costed choice.
- Because approvals are refused, a case that needs a tool that asks fails with that reason rather than hanging.
