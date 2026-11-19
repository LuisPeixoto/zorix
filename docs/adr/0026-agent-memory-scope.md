# ADR-0026: Agents get a memory scope of their own

Status: accepted (Phase 5)

## Context

An agent that serves one purpose should be able to remember things only it needs, without those facts reaching every other
run. Memory is already scoped (global, project, conversation) and everything remembered is approved by a person.

## Decision

- A fourth scope, `agent`, stored as `("agent", <agent id>)`. The memory table keeps its columns.
- `Visibility` gains the agent. A run of an agent sees global, project, conversation and **its own** agent facts. A run that
  is not an agent's, and runs of other agents, never see them.
- `memory.remember` accepts `scope: "agent"` only inside an agent run; it still needs a person's approval and no automatic mode
  gives it. The CLI takes `--scope agent:<slug>`; the Memory screen can filter by agent.
- Deleting an agent removes its memories after asking; archiving keeps them.

## Consequences

- A database written with agent facts is not readable by a 0.4 binary (schema 4 guards that).
- Agent memory is per agent, not per version: improving instructions does not lose what was learned.
