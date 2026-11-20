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

## ✅ Phase 2 — AI Provider Engine · v0.2.0

Real models behind `ModelGateway`: [adapters for Anthropic, OpenAI (and compatible servers), Gemini and Ollama](providers.md)
· a curated model catalogue with discovery · [credentials in the operating system's keychain](providers.md#credentials) ·
[routes, aliases and fallback](providers.md#fallback) with cooldowns and `local_only` · the context budget taken from the model's
real window · [native turn replay](adr/0012-native-turns-append-only.md) · token, cache and reasoning usage with cost when the price
is known · the provider and model of every step · `zorix models`, `provider` and `route` and a providers screen in the desktop
app · [live tests](testing.md#live-tests) against Ollama and, with your keys, the cloud.

Not yet: live text while the model writes, a classifier-based router, embeddings, batch APIs.

## ✅ Phase 3 — Context & Memory Engine · v0.3.0

Memory as new context sources: the **conversation**, **facts and preferences** that a person approves, **documents** (text, code,
HTML and PDF folders, with citations) and **past work**, found by keyword search (SQLite FTS5) and, when you choose an embeddings
model, by meaning (Ollama, OpenAI, Gemini). What was retrieved is frozen per execution. Token counts are learned per model from the
usage providers report. Scopes: global, project, conversation. Guides: [memory](memory.md), [knowledge](knowledge.md).

Not yet: summarising long conversations, a file watcher, OCR, memory shared between people, an on-disk vector index for very
large collections.

## ✅ Phase 4 — Tool & Integration Engine · v0.4.0

- A **Model Context Protocol client** for both protocol eras, over stdio and Streamable HTTP with OAuth 2.1, with tool pinning
  and review, a catalogue, and import from Claude Desktop, VS Code and Cursor ([MCP](mcp.md)).
- **Native integrations**: generic HTTP, outbound signed webhooks, GitHub, GitLab, Jira, Slack, Discord, SQLite and Postgres,
  behind per-resource grants and approvals ([integrations](integrations.md)).
- A **WebAssembly sandbox** for JavaScript and Python ([sandbox](sandbox.md)).
- **`zorix mcp serve`**: ZORIX's memory, read-only, for other MCP clients.
- The CLI (`mcp`, `connect`, `sandbox`, run flags), the **Integrations screens**, approval cards per kind of call, a command
  palette and design system v2.

Later: sandboxing of stdio MCP servers, MCP sampling and elicitation, inbound webhooks (with workflows), OAuth for the native
integrations, more catalogue entries, cloud providers' APIs.

## ✅ Phase 5 — AI Agent Engine · v0.5.0

- **Agents**: identity, instructions, skills, tools and access, memory of their own and authorised models, independent of the model that runs them. Stored in the
  database, versioned (every run records the version it used) and exported and imported as one Markdown file ([agents](agents.md)).
- **Skills** in the open Agent Skills format: a folder with a `SKILL.md`, loaded on demand, scripts only in the sandbox ([skills](skills.md)).
- An **agent memory scope**, approved by a person like every memory ([memory](memory.md)).
- A **model allow-list** the router cannot escape, and **live text** that appears as an answer is written.
- **Evaluations**: deterministic checks, an optional judge, comparison between versions and models ([evals](evals.md)).
- Four **starter agents** with skills and evaluation suites; agents over **MCP** (`zorix.agents`, `zorix.run_agent`).
- The CLI (`agent`, `skill`, `eval`, `run --agent`), and the **Agents screens**: gallery, chat, definition, skills, versions, evals and memory.

Later: skills that need native packages, reasoning-token streaming.

## ✅ Phase 6 — Multi-Agent Orchestration · v0.6.0

- **Delegation**: the `agents.delegate` tool starts child runs, up to eight at once, three levels deep. A parent waits in a new state, `waiting_for_children`,
  without holding a worker, and receives each child's answer, structured result, steps and cost ([orchestration](orchestration.md)).
- **Budgets and permissions only shrink**: a child's grants are its agent's grants intersected with the parent's, the remaining budget is split among the children,
  and what they spend rolls up. Loops, archived agents and models the parent may not use are refused with a reason.
- **Approvals rise to the root**: a sub-run's approval stops the tree and is answered from the run you started. Cancelling the root cancels the tree.
- **Teams**: a lead and the agents it may ask, run from one place, with a ready-made *research desk* ([teams](teams.md)).
- The CLI (`team`, `run --team`, `tree`, roots-only `list`), **MCP** (`team` on `zorix.run_agent`), and the app: delegation cards in the chat, a work tree in the run view,
  the Teams screens, and teams in New run and ⌘K. Database schema 5.

Later: adaptive budget splits, a scratchpad shared by siblings, team versions and a team file format.

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
