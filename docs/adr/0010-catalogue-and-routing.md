# ADR-0010: A curated model catalogue and declarative routes with fallback

Status: accepted (Phase 2)

## Context

A task should be able to name a model, an alias, or a policy ("the cheapest model that supports tools and fits this
context"), and keep working when a provider fails. A classifier model that picks the model would add cost and
unpredictability to every execution.

## Decision

- A **catalogue** (`models.json`, with an "as of" date, overridable in `providers.toml`) lists provider, model id,
  context window, output limit, capabilities (tools, vision, thinking), prices and a latency class.
- Models are referenced as `provider/model`, by **alias**, or as `route:<name>`. A **route** is an ordered list of
  candidates plus constraints (capabilities required, `local_only`, cost ceiling).
- The router filters candidates by capability and context fit, orders them as the route says, and **falls back** on
  unavailable, rate limited, auth, not found, context too long and refused. It never retries a call that cannot succeed
  as it is (invalid request) and never falls back after the caller cancelled.
- Every switch is an event (`execution.model.fallback`) with the reason; the response carries the route trace.

## Consequences

- Behaviour is deterministic and testable with fake gateways.
- Prices can be wrong or missing; cost is reported as unknown rather than guessed.
- Smarter routing (a classifier, learned preferences) can be added later as another way to produce the candidate order.
