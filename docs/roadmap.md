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

## Design system v3 · v0.6.1

The app follows the [design system v3](design-system.md) strictly: tokens generated from `design/tokens.json`, a design lint in the gates, the kit matched to its
specifications, and the shell, Runs, Run, New run, Providers, Memory, Documents, Integrations, Agents, Teams, Agent chat and Settings rebuilt to the assembled screens in
`design/screens/`. New: an **Inbox** with everything waiting for a person, a Runs page, and spend and running counts in the sidebar ([ADR-0032](adr/0032-design-system-v3-is-the-contract.md)).

Every later phase below lists its screens and the design system components they are built from (from `design/surfaces.md`). A screen the design system lacks is added there first.

## ✅ Phase 7 — Workflow Engine · v0.7.0

Visual workflows with conditions, parallel branches, human approvals, schedules, webhooks and events as triggers. Versioned and reusable. Guide: [workflows](workflows.md). Shipped:
the model and store (schema 6), the engine and host (`zorix-workflows`, `zorix serve`), `zorix workflow`, and the Workflows screens in the app.
Screens: Workflows, Workflow editor, workflow run view. Built from WorkflowCanvas, WorkflowNode, TriggerChip, Drawer, VersionTimeline and ApprovalCard.

- **Triggers:** a schedule ("Weekdays at 08:00"), a webhook (for example a GitHub issue labelled `incident`), and an event (another workflow finishing, a new ticket).
- **Steps:** an agent or a team; logic (condition, loop, wait); people (an **approval** step, and a step that asks a question); actions (send a message, open a pull request, an HTTP request).
- **Approval step:** who decides (a person or an on-call rotation), what they are shown (summary, severity, diff, cost), a time-out and what happens then (refuse, or continue), and a reminder in Slack.
- **Editor:** a canvas with a palette of triggers, work, logic, people and actions; an inspector for the selected step with the last runs of that step and its median wait; zoom and fit.
- **Versions:** a draft beside the live version, a **test run**, and **Publish vN**, shown with VersionTimeline and compared with DiffView.
- **List:** active, paused, runs this week and success rate, workflows waiting for people, time saved; each row shows what starts it, the last 14 runs, the version and the state, with an on/off switch and templates.
- **In New run:** *Run as Workflow* and *Save as workflow*.

Decisions: [ADR-0033](adr/0033-workflows-are-versioned-graphs-that-create-executions.md) (versioned graphs that create executions),
[ADR-0034](adr/0034-the-workflow-host-and-triggers.md) (the app or `zorix serve` hosts the triggers; webhooks are off by default) and
[ADR-0035](adr/0035-people-steps-and-unattended-actions.md) (people steps; unattended actions are confirmed when publishing).
The screens add *Workflow run* to the design system, logged in `design/local-additions.md`.

Later: a **pull request** action (no tool creates branches or commits yet), the **new ticket** event (needs polling of integrations),
an **on-call rotation** as the approver (needs people and accounts, Phase 10), and per-period workflow budgets.

## ✅ Phase 8 — Task & Project Management · v0.8.0

Projects, tasks, assignment to people or agents, priorities, dependencies, deadlines, history and deliverables.
Screens: Projects, board, list, timeline, task. Built from TaskBoard, TaskCard, Avatar, Drawer and Stepper.

- **Projects** with an owner, an agent manager, a due date and progress ("18 of 29 tasks"), shared with people.
- **Tasks** with a key (`OPS-38`), priority (urgent, high, normal, low), assignee (a person or an agent), progress, due date, links (a pull request), and a *Needs you* flag when a person must act.
- **Views:** Board, List and Timeline (bars fill with progress; dependencies are drawn), plus Deliverables and History, with filters for everyone, agents, people, and urgent and high.
- A task moving to Done under a manager asks for approval like any other change.

Guide: [projects](projects.md). Shipped: the model and store (schema 7), `zorix-projects`, `zorix project` and `zorix task`, and the Projects screens in the app.

Decisions: [ADR-0036](adr/0036-projects-and-tasks-are-records-that-create-runs.md) (projects and tasks are records that create runs; fixed lanes; *Needs you* is derived)
and [ADR-0037](adr/0037-project-tools-and-the-manager.md) (project tools and the manager; a move to Done always asks).
The screens add *Projects*, the project's *List*, *Deliverables* and *History*, *Task* and *New project* to the design system, and dependency arrows to the timeline,
logged in `design/local-additions.md`. Until accounts exist (Phase 10) the only person is you, so there is no *Share*.

Later: **people and sharing** (Phase 10), custom lanes, recurring tasks, a workflow step and trigger for tasks, import and export of a project, and the day a task finished.

## Phase 9 — AI Workforce Management

Virtual employees, departments, hierarchies and virtual managers that plan, assign, monitor and report within configurable autonomy.
Screens: Organisation (org chart, list, activity), worker profile, autonomy. Built from OrgChart, WorkerCard, AutonomyDial, MetricTile and LogStream.

- **Workers** (agents and people) in departments, with who they report to; a virtual manager plans the week, assigns tasks and reports on Fridays.
- **Autonomy** in five levels: Suggest, Ask, Notify, Budgeted and Full, with a weekly budget and a time limit per task; writes still ask.
- A worker's tasks, on-time rate, eval score, weekly spend and activity; *Hire a worker* and *Org settings*; a weekly spend against a budget.

## Phase 10 — Command Center

The complete interface: dashboards, monitoring, logs and observability, reports, settings and permissions.
Screens: Overview, Logs and traces, Settings (permissions and policy, members, secrets, retention, updates). Built from MetricTile, ChartPanel, LogStream, HealthGrid, ToggleRow and KeyValue.

- **Overview:** runs today, **spend against a daily budget**, runs waiting for people, success rate, spend per day against the previous period, spend by provider, the waiting queue, health with 24 hours of history, a live log, periods (today, 7 and 30 days) and *Export report*.
- **Logs and traces:** a live view with filters by source and level, a trace waterfall for a run or workflow, the selected line with its span and run, secrets redacted before anything is stored, export and alerts.
- **Settings:** permission and policy rules (ask first, allow or deny, with the hits of the week), the approval switches ("Always for this run" off for databases and money), members and roles (Owner, Approver, Viewer), secrets, retention and updates.
- Per-provider latency (p95) and spend; today's budget in the sidebar.

## Phase 11 — Marketplace & Ecosystem

Sharing agents, skills, integrations and workflows.
Screens: Browse, listing, publish. Built from ListingCard, Tabs, Stepper, Callout and Modal.

- Search, categories (agents, teams, workflows, skills, integrations), sorting, ratings and install counts, and the publisher.
- Every item lists the permissions it asks for before install, and nothing is granted until a person says so.
- The listing page and the publish flow.

## Across phases

- **Signed releases and automatic updates** (see [distribution](distribution.md)).
- **Accounts, organisations and a shared store** for teams, behind the existing repository trait (the tenant is already part of every record).
- **OpenTelemetry export** once the GenAI conventions stabilise.
- **Retention policies** for stored executions.
- A **cost and time estimate** before a run starts, **attachments** with `@` and drop in New run, and **accounts and avatars** for people.

## Ideas

Not planned yet: a Temporal or Postgres backend for team deployments; an MCP server that exposes ZORIX itself; replay of
an execution against another model for comparison; evaluation suites for answer quality.
