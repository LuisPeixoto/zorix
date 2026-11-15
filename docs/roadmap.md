# Roadmap

ZORIX is built in phases. Each phase ends in a release with acceptance criteria, and each is designed to work alone and
plug into the rest. The order follows what depends on what: the runtime first, because everything else runs on it.

## ✅ Phase 1 — Harness Runtime · v0.1.0

The execution core: [executions with a strict lifecycle](runtime.md#the-lifecycle) · [the loop with limits, retries,
timeouts, cancel, pause and resume](runtime.md#the-loop) · [a context builder with sources and a budget](adr/0007-context-builder.md)
· [a tool registry and executor with schema validation, timeouts and output caps](tools.md) · [deny-by-default grants, a policy
engine, human approval bound to one call, and prompt-injection containment](permissions.md) · [ordered, replayable events](events.md) ·
[SQLite persistence with checkpoints, leases, idempotency and crash recovery](persistence.md) · [GenAI-named trace spans, secret
redaction and an execution summary](permissions.md#credentials) · simulated models · built-in demo tools with path confinement ·
[the `zorix` command line](cli.md) · [a minimal desktop app](desktop.md) · [CI on three operating systems and a release
workflow](distribution.md) · [documentation, ADRs and a technology comparison](research/runtime-landscape.md).

## Phase 2 — AI Provider Engine · v0.2.0

Real models behind `ModelGateway`: a model catalogue (provider, id, credentials, parameters, capabilities, limits, cost,
latency, context window, tool and multimodal support), provider adapters (HTTP APIs, SDKs, local runtimes), a credential store
backed by the operating system's keychain, per-model token counting and pricing, and **routing** by task, complexity, cost
and speed, with fallbacks. Several models in one execution.

## Phase 3 — Context & Memory Engine

Memory as new `ContextSource`s: short-term (the execution), long-term, operational (what was done and decided), knowledge
(documents) and shared memory, with retrieval, freshness, conflict handling and user control. Real token budgeting per model.

## Phase 4 — Tool & Integration Engine

A Model Context Protocol client, HTTP and webhook tools, integrations (GitHub, GitLab, Jira, databases, cloud, chat),
credential handling, and a sandbox for tools that run code. Tool permissions grow per integration.

## Phase 5 — AI Agent Engine

Agents ("AI employees"): identity, instructions, skills, tools, memory and authorised models, independent of the model that
runs them. Skill libraries. Agents create executions with their own grants and limits.

## Phase 6 — Multi-Agent Orchestration

Delegation, parallel work and structured context transfer. An execution can start child executions (the parent id is
already part of the model), with budgets and permissions that can only shrink down the tree.

## Phase 7 — Workflow Engine

Visual workflows with conditions, parallel branches, human approvals, schedules, webhooks and events as triggers.
Versioned and reusable.

## Phase 8 — Task & Project Management

Projects, tasks, assignment to people or agents, priorities, dependencies, deadlines, history and deliverables.

## Phase 9 — AI Workforce Management

Virtual employees, departments, hierarchies and virtual managers that plan, assign, monitor and report within configurable
autonomy.

## Phase 10 — Command Center

The complete interface: dashboards, monitoring, logs and observability, reports, settings and permissions.

## Phase 11 — Marketplace & Ecosystem

Sharing agents, skills, integrations and workflows.

## Across phases

- **Signed releases and automatic updates** (see [distribution](distribution.md)).
- **Accounts, organisations and a shared store** for teams, behind the existing repository trait (the tenant is already part of every record).
- **OpenTelemetry export** once the GenAI conventions stabilise.
- **Retention policies** for stored executions.

## Ideas

Not planned yet: a Temporal or Postgres backend for team deployments; an MCP server that exposes ZORIX itself; replay of
an execution against another model for comparison; evaluation suites for answer quality.
