# Roadmap

ZORIX is built in phases. Each phase ends in a release with acceptance criteria, and each is designed to work alone and
plug into the rest. The order follows what depends on what: the runtime first, because everything else runs on it.

Phases 1 to 9 built the product. Phases 10 to 15 make sure what was built meets the [engineering guidelines](engineering-guidelines.md),
fix what that finds, and complete the features that were deferred. The Command Center and the Marketplace come after them,
on a base that has been audited.

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

Live text and embeddings arrived in Phases 5 and 3. Later: a classifier-based router and image or audio generation (see [Ideas](#ideas)), and batch APIs for evaluations (Phase 13).

## ✅ Phase 3 — Context & Memory Engine · v0.3.0

Memory as new context sources: the **conversation**, **facts and preferences** that a person approves, **documents** (text, code,
HTML and PDF folders, with citations) and **past work**, found by keyword search (SQLite FTS5) and, when you choose an embeddings
model, by meaning (Ollama, OpenAI, Gemini). What was retrieved is frozen per execution. Token counts are learned per model from the
usage providers report. Scopes: global, project, conversation. Guides: [memory](memory.md), [knowledge](knowledge.md).

Later: summarising long conversations, a file watcher, OCR and an on-disk vector index for very large collections (Phase 11); memory shared between people (Phase 16).

## ✅ Phase 4 — Tool & Integration Engine · v0.4.0

- A **Model Context Protocol client** for both protocol eras, over stdio and Streamable HTTP with OAuth 2.1, with tool pinning
  and review, a catalogue, and import from Claude Desktop, VS Code and Cursor ([MCP](mcp.md)).
- **Native integrations**: generic HTTP, outbound signed webhooks, GitHub, GitLab, Jira, Slack, Discord, SQLite and Postgres,
  behind per-resource grants and approvals ([integrations](integrations.md)).
- A **WebAssembly sandbox** for JavaScript and Python ([sandbox](sandbox.md)).
- **`zorix mcp serve`**: ZORIX's memory, read-only, for other MCP clients.
- The CLI (`mcp`, `connect`, `sandbox`, run flags), the **Integrations screens**, approval cards per kind of call, a command
  palette and design system v2.

Inbound webhooks arrived with workflows (Phase 7). Later (Phase 12): sandboxing of stdio MCP servers, MCP sampling and elicitation, OAuth for the native
integrations, more catalogue entries. Cloud providers' APIs are in [Ideas](#ideas).

## ✅ Phase 5 — AI Agent Engine · v0.5.0

- **Agents**: identity, instructions, skills, tools and access, memory of their own and authorised models, independent of the model that runs them. Stored in the
  database, versioned (every run records the version it used) and exported and imported as one Markdown file ([agents](agents.md)).
- **Skills** in the open Agent Skills format: a folder with a `SKILL.md`, loaded on demand, scripts only in the sandbox ([skills](skills.md)).
- An **agent memory scope**, approved by a person like every memory ([memory](memory.md)).
- A **model allow-list** the router cannot escape, and **live text** that appears as an answer is written.
- **Evaluations**: deterministic checks, an optional judge, comparison between versions and models ([evals](evals.md)).
- Four **starter agents** with skills and evaluation suites; agents over **MCP** (`zorix.agents`, `zorix.run_agent`).
- The CLI (`agent`, `skill`, `eval`, `run --agent`), and the **Agents screens**: gallery, chat, definition, skills, versions, evals and memory.

Later (Phase 13): skills that need native packages, reasoning-token streaming.

## ✅ Phase 6 — Multi-Agent Orchestration · v0.6.0

- **Delegation**: the `agents.delegate` tool starts child runs, up to eight at once, three levels deep. A parent waits in a new state, `waiting_for_children`,
  without holding a worker, and receives each child's answer, structured result, steps and cost ([orchestration](orchestration.md)).
- **Budgets and permissions only shrink**: a child's grants are its agent's grants intersected with the parent's, the remaining budget is split among the children,
  and what they spend rolls up. Loops, archived agents and models the parent may not use are refused with a reason.
- **Approvals rise to the root**: a sub-run's approval stops the tree and is answered from the run you started. Cancelling the root cancels the tree.
- **Teams**: a lead and the agents it may ask, run from one place, with a ready-made *research desk* ([teams](teams.md)).
- The CLI (`team`, `run --team`, `tree`, roots-only `list`), **MCP** (`team` on `zorix.run_agent`), and the app: delegation cards in the chat, a work tree in the run view,
  the Teams screens, and teams in New run and ⌘K. Database schema 5.

Later (Phase 13): adaptive budget splits, a scratchpad shared by siblings, team versions and a team file format.

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
per-period workflow budgets and sub-workflows (Phase 14, on the tools of Phase 12), and an **on-call rotation** as the approver (needs people and
accounts, Phase 16).

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
logged in `design/local-additions.md`. Until accounts exist (Phase 16) the only person is you, so there is no *Share*. A task now remembers the day it went to Done (0.9.0).

Later: **people and sharing** (Phase 16), custom lanes, recurring tasks, a workflow step and trigger for tasks, and import and export of a project (Phase 14).

## ✅ Phase 9 — AI Workforce Management · v0.9.0

Virtual employees, departments, hierarchies and virtual managers that plan, assign, monitor and report within configurable autonomy.
Screens: Organisation (org chart, list, activity), worker profile, autonomy. Built from OrgChart, WorkerCard, AutonomyDial, MetricTile and LogStream.

- **Workers** (agents and people) in departments, with who they report to; a virtual manager plans the week, assigns tasks and reports on Fridays.
- **Autonomy** in five levels: Suggest, Ask, Notify, Budgeted and Full, with a weekly budget and a time limit per task; writes still ask.
- A worker's tasks, on-time rate, eval score, weekly spend and activity; *Hire a worker* and *Org settings*; a weekly spend against a budget.

Guide: [workforce](workforce.md). Shipped: the model and store (schema 8), `zorix-workforce`, the autonomy levels in the policy, `zorix org` and `zorix worker`, and the Workforce screens in the app.

Decisions: [ADR-0038](adr/0038-workers-are-records-over-agents.md) (workers are records over agents; chains end at you; trees shrink; figures are derived)
and [ADR-0039](adr/0039-autonomy-is-decided-by-the-policy.md) (autonomy is decided by the policy; what still asks; budgets are narrowings).
The screens add the Workforce *List* and *Activity* tabs, *Worker*, *Hire a worker* and *Org settings* to the design system, logged in `design/local-additions.md`.

Later: people other than you and hiring them (Phase 16), a **daily** budget (Phase 16, with the Command Center), and moving workers between departments by drag, teams as workers and importing an org chart (Phase 14).

## How a quality phase works

Phases 11 to 15 each take one area of the product through the same steps, so nothing is left to luck:

1. **Audit.** Check the area against sections 2 to 14 of the [guidelines](engineering-guidelines.md): tests, completeness, UX, architecture, security, performance, errors, documentation, compatibility, accessibility, observability, AI quality and dependencies. The result is `docs/audits/<area>.md`, with a severity for every finding.
2. **Issues.** Every finding becomes an issue in the phase's milestone. Security findings are fixed first.
3. **Fixes.** Each fix is its own branch and carries a regression test that fails without it.
4. **Completion.** Then the area's deferred features are built, each with its tests, documentation and a security review where it touches permissions, tools or execution.
5. **Close.** The phase ends with the [final approval criteria](engineering-guidelines.md#17-final-approval-criteria), the documentation checked against the code, and a release.

## Phase 10 — Quality foundation · v0.10.0

The tools every audit needs, and the gaps already known.

- **End-to-end tests of the built app** ([#2](https://github.com/LuisPeixoto/zorix/issues/2)): first run, a run with the demo model, an approval, a project task and the workforce screens, on a throwaway `ZORIX_HOME`, with a screenshot attached to a failure. Linux and Windows drive the app through `tauri-driver`; macOS has no WebDriver for its web view, so it gets a launch smoke test.
- **Interface strings in one catalogue** ([#1](https://github.com/LuisPeixoto/zorix/issues/1)): English as the source, every component reading from it, a lint that rejects user-facing literals, one locale-aware formatter for dates, times, numbers and money, and a pseudo-language of longer, accented text in the tests to catch cut-off words.
- **Scale benchmarks:** 10,000 executions, 1,000,000 events, 200 agents (the most a tenant keeps), 5,000 facts and 50,000 pieces of documents, timing listing, replay, search and the Inbox. The numbers go into [testing](testing.md#benchmarks), and a large regression fails.
- **An adversarial suite:** prompt injection at every untrusted input (tool results, documents, MCP descriptions, webhook payloads, memory, task text), tampering with approvals, path traversal, SSRF in the HTTP integration, child runs that try to widen their grants, and fuzz targets for the parsers run on a schedule.
- **Fault injection:** provider outages and timeouts, a crash during a commit, a full disk and concurrent writers, each followed by a resume.
- **Coverage** reported per crate in CI, published, with no artificial target.
- **Installers verified on clean machines** ([#3](https://github.com/LuisPeixoto/zorix/issues/3)), with a checklist for each release in [distribution](distribution.md).
- **Replace `chrono`** ([#17](https://github.com/LuisPeixoto/zorix/issues/17)) with a maintained date library; every schedule test passes unchanged.
- **Documentation drift:** every "not yet" and "later" claim in the guides is checked against the code.

## Phase 11 — Runtime, providers and memory · v0.11.0

Audit and complete Phases 1 to 3.

- **Audit focus:** crash recovery under concurrency, lease and idempotency races, provider failure and fallback with each adapter, token and cost accounting, redaction on every path that stores text, and memory isolation between tenants and scopes.
- **Completion:** summarising long conversations instead of dropping the oldest turns · **retention policies** for stored executions · a **cost and time estimate** before a run starts · **OpenTelemetry export** with the GenAI conventions · a **file watcher** for document folders · optional local **OCR** for scanned PDFs · an on-disk vector index for large collections · **attachments** with `@` and drag and drop in New run.

## Phase 12 — Tools, integrations and sandbox · v0.12.0

Audit and complete Phase 4.

- **Audit focus:** SSRF and redirects in HTTP, path confinement, MCP conformance for both protocol eras, OAuth token storage and refresh, sandbox escape tests on all three systems (including the Windows WASI case fixed in 0.9.x), and output caps.
- **Completion:** sandboxing of stdio MCP servers · MCP **sampling** and **elicitation** · **OAuth** for the native integrations · more catalogue entries · a **git tool** (branch, commit, push, through grants and approvals) · a **pull request** tool for GitHub and GitLab · **polling** of integrations for new events, which the "new ticket" workflow event needs · an **MCP server that exposes ZORIX itself** (runs, approvals, agents), beside the read-only memory.

## Phase 13 — Agents, teams and orchestration · v0.13.0

Audit and complete Phases 5 and 6.

- **Audit focus:** delegation limits and budget roll-up under failure, approvals up the tree, children never widening grants, the agent file format round-trip, behavioural regressions when instructions, models or tools change (evaluation suites run in CI on the simulated models), and a test matrix over several models.
- **Completion:** **reasoning-token streaming** · skills that need native packages, inside the sandbox · **adaptive budget splits** · a **scratchpad shared by siblings** · **team versions** and a team file format · **replaying an execution against another model** to compare · **answer-quality evaluation suites** · batch APIs for running evaluations at lower cost.

## Phase 14 — Workflows, projects and workforce · v0.14.0

Audit and complete Phases 7 to 9.

- **Audit focus:** schedules and the missed-fire rule across time zones and daylight saving, webhook signing, unattended actions, the autonomy table at every level, spend figures that never count twice, and migrations 6 to 8 with real data.
- **Completion:** the **pull request** action and the **new ticket** event (on the tools of Phase 12) · per-period workflow budgets and **sub-workflows** · **custom lanes**, **recurring tasks**, a **workflow step and trigger for tasks** and **import and export of a project** · moving workers between departments by dragging, **teams as workers** and **importing an org chart**.

## Phase 15 — Interface and languages · v0.15.0

Every screen audited and finished.

- **Layout:** responsive at the smallest window, 1280, 1440 and very large sizes, with nothing cut, overlapping or off screen.
- **States:** loading, empty, error, success and disabled on every screen.
- **Accessibility:** keyboard paths, visible focus, screen-reader labels and contrast, with a design critique and an accessibility review whose findings are fixed. Light and Dark screenshots beside the design system.
- **Languages:** full translations into **Português (Brasil)**, **Español** and **简体中文**, with a language picker in Settings and each language's date and number formats; font fallback and text-width checks for Chinese; a CI check that every key is translated; a glossary per language; and an invitation to native speakers to review through Discussions.

## Phase 16 — Command Center · v0.16.0

The complete interface: dashboards, monitoring, logs and observability, reports, settings and permissions.
Screens: Overview, Logs and traces, Settings (permissions and policy, members, secrets, retention, updates). Built from MetricTile, ChartPanel, LogStream, HealthGrid, ToggleRow and KeyValue.

- **Overview:** runs today, **spend against a daily budget**, runs waiting for people, success rate, spend per day against the previous period, spend by provider, the waiting queue, health with 24 hours of history, a live log, periods (today, 7 and 30 days) and *Export report*.
- **Logs and traces:** a live view with filters by source and level, a trace waterfall for a run or workflow, the selected line with its span and run, secrets redacted before anything is stored, export and alerts.
- **Settings:** permission and policy rules (ask first, allow or deny, with the hits of the week), the approval switches ("Always for this run" off for databases and money), members and roles (Owner, Approver, Viewer), secrets, retention and updates.
- Per-provider latency (p95) and spend; today's budget in the sidebar.
- **Accounts, people and roles** (Owner, Approver, Viewer), behind the existing repository trait (the tenant is already part of every record). They unlock what earlier phases deferred: *Share* on projects, hiring people as workers, on-call rotations as workflow approvers, memory shared between people, and the **daily budget**.

## Phase 17 — Marketplace & Ecosystem · v0.17.0

Sharing agents, skills, integrations and workflows.
Screens: Browse, listing, publish. Built from ListingCard, Tabs, Stepper, Callout and Modal.

- Search, categories (agents, teams, workflows, skills, integrations), sorting, ratings and install counts, and the publisher.
- Every item lists the permissions it asks for before install, and nothing is granted until a person says so.
- The listing page and the publish flow.

## Across phases

- **Signed releases and automatic updates** (see [distribution](distribution.md)).
- **Organisations and a shared store** for teams, behind the existing repository trait (the tenant is already part of every record). People and roles arrive with the Command Center (Phase 16).

## Ideas

Not planned yet: a Temporal or Postgres backend for team deployments; a classifier-based router; image and audio generation; the cloud providers' own APIs as integrations.
