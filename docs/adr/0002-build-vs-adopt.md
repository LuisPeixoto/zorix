# ADR-0002: Build a small durable runtime, reuse mature crates

Status: accepted (Phase 1)

## Context

See `docs/research/runtime-landscape.md`. Agent frameworks are Python, TypeScript or .NET and let the model drive control
flow. Temporal has a GA Rust SDK but needs a server. ZORIX is a desktop-first Rust application whose differentiator is
governed execution.

## Decision

Build the lifecycle, loop, policy, approvals and repository ourselves, using ideas from LangGraph and Inngest
(checkpoint per step), OpenAI, ADK and Mastra (approval as suspension) and Temporal (idempotent steps). Reuse `tokio`,
`serde`, `jsonschema`, `rusqlite`, `tracing`, `uuid`, `sha2`, `clap`.

## Consequences

- We own the correctness of the state machine and recovery, and test it heavily.
- Provider SDKs (Phase 2) and an MCP client (Phase 4) plug in behind our traits.
- A Temporal or Postgres backend stays possible behind `ExecutionRepository` and `ExecutionScheduler`.
