# ADR-0024: Agents are versioned definitions that create executions

Status: accepted (Phase 5)

## Context

Every run is assembled by hand: prompt, instructions, tools, grants, model, limits. People want a named, reusable "employee" that
carries those, and want to know exactly which definition produced a result.

## Decision

- An **agent** is a slug plus a sequence of **versions**. A version is an immutable `AgentSpec`: name, description, avatar,
  instructions, skills (pinned by content hash), tool patterns, access (the grant syntax of `--allow-…`), models (a default and
  an allow-list), limits, memory options, generation parameters, an optional expected-output schema, and metadata.
- Saving writes version N+1 only when the SHA-256 of the canonical JSON of the spec changed. Rollback writes a **new** version
  that copies an old one. History is never rewritten.
- An **AgentRunner** turns a version and a request into a `NewExecution`: the model must be allowed; instructions are the agent's
  plus an index of its skills; grants are the agent's, and a run may only **narrow** them (`Grants::intersect`: whatever is
  allowed is allowed by both sides); limits are the field-by-field minimum; memory options the field-by-field AND.
- The execution carries a typed **`AgentBinding`** `{agent, slug, version, hash, skills, allowed_models}`, frozen at creation
  like the tool pins, and mirrored on `ToolContext` so tools can see it. Metadata would not do: the runtime and memory need a
  typed value.
- Agents export and import as Markdown with YAML frontmatter and the instructions as the body. Export is deterministic.
  Importing an existing slug adds a version.
- Approvals do not change: anything that changes something asks a person, whatever the agent.

## Consequences

- A run is reproducible from its binding: the version and the skill hashes are recorded.
- A client of the same tenant could send a forged binding. Acceptable while ZORIX is local; engine-side validation against the
  store comes with multi-user.
- Database schema 4 adds the tables; an older binary refuses the newer database.
