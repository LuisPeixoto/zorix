# Workflows

A workflow is a graph of **triggers** and **steps** that runs by itself and stops where a person must decide. It starts an agent or a team, branches on what they said,
waits for an approval, and sends a message. Decisions: [ADR-0033](adr/0033-workflows-are-versioned-graphs-that-create-executions.md) (graphs and versions),
[ADR-0034](adr/0034-the-workflow-host-and-triggers.md) (the host and triggers) and [ADR-0035](adr/0035-people-steps-and-unattended-actions.md) (people and unattended actions).

A workflow adds no new way to run a model. An agent or team step creates an **ordinary run**, with its tree, approvals, cost and events, linked back by the metadata
`workflow_run` and `workflow_step`. The run view shows *From workflow …* with a link.

## The shape

- **Triggers** start it: a schedule, a webhook, an event, another workflow ending, or a person (manual). A workflow has at least one.
- **Steps** do the work. At most 50. An arrow joins one step to the next. The graph goes one way: no loops (use a *loop* step to repeat).
- A step with several inputs waits for the inputs that were not skipped. That is how parallel branches join.
- A **condition** has two arrows, `yes` and `no`. The branch not taken is *skipped*, and the skip passes on until a step has an input that ran.
- Up to 8 steps run at the same time.

| Kind | What it does |
|---|---|
| Schedule | Every N minutes (at least 5), at times on chosen days ("Weekdays 08:00"), or a cron expression, in a time zone (the computer's by default) |
| Webhook | Starts when a request arrives. A *filter* (a condition over the body) ignores requests; *signed* requires `X-Hub-Signature-256` |
| Event | An agent or team run ended well, failed, or either, optionally one agent. Runs that workflows started do not start workflows |
| After workflow | Another workflow finished, failed or ended. A chain is cut at 4, and a workflow never follows itself |
| Manual | `zorix workflow run`, *Test run*, the list, or *Run as › Workflow* in New run. What you write there arrives as `trigger.payload.prompt` |
| Agent, Team | Ask an agent (a version, a model and a cost limit are optional) or a team through its lead |
| Condition | A yes/no expression |
| Loop | For each item of a list (at most 8), ask an agent |
| Wait | For a while, or until a time of day. A durable timer |
| Approval | Stops until a person approves. Shows chosen values; a time-out, then *refuse* or *continue*; an optional reminder in Slack |
| Ask a question | Stops until a person answers in words; a default answer and a time-out are optional |
| Send message | Slack or Discord |
| HTTP request | `GET`, `POST`, `PUT`, `PATCH` or `DELETE` |

Not yet: a pull request action (no tool creates branches or commits), a "new ticket" event, an on-call rotation as the approver, and workflow budgets per period.

## Values and expressions

Steps pass values with `{{path}}`: `{{trigger.payload.issue.title}}`, `{{steps.triage.output.text}}`, `{{run.cost_micros}}`, and inside a loop `{{item}}` and `{{index}}`.
A path that points at nothing is an **error, never an empty string**, so a typo cannot send an empty message. A step can only read steps that run before it, and ZORIX checks
this when you save.

What a step writes as `output`: an agent or team, `text` and `structured`; a loop, `items`; a condition, `value`; an approval, `approved` and `by`; a question, `answer`.

Conditions and webhook filters use a small language: paths, numbers, `"text"`, `true`, `false`, `null`, `== != < <= > >=`, `contains` (text, case-insensitive, or a list),
`and`, `or`, `not` and parentheses. Nothing else: no calls, no assignment. An expression always ends and can only read.

## Versions

A workflow has one **draft** (what the editor changes, saved a moment after each edit) and immutable **published versions**. *Publish vN* makes a new version; the next
trigger uses it, and runs that are going keep the version they started with. A move on the canvas is not a change. Rolling back publishes an old definition again as a new
version. *Versions* shows a diff of the readable outline (one fact per line, positions left out). A draft can be invalid while you work; it says why it was not saved and keeps
your edits.

A **test run** runs the draft, is marked as a test, and **asks before every action**. It does not count in the list.

## People

An approval or a question is a **wait**, not tied to a tool call. It shows exactly what the step chose to show, with a **fingerprint** of it. The answer names that fingerprint, so
it cannot be applied to anything else, and a wait can be answered once. Waits appear in the **Inbox** and in the run view. A time-out follows the step: refuse, or go on
(an approval counts as approved; a question takes its default answer).

An agent step's own tool approvals still rise to its run, as in [orchestration](orchestration.md); the step shows *waiting for a person*.

**Actions ask first by default.** A message or request step makes a wait showing the tool and its arguments, and nothing is sent until a person agrees. A step set to *runs without
asking* (`ask: never`) needs a **confirmation when publishing**: the version records who confirmed, when, and the hash of each step. Editing a confirmed step drops its confirmation.
A reminder in Slack on an approval counts as unattended too. Resource grants still apply: the message goes only to the channel the step names, and a request only to the host in its address.

If ZORIX stops while an action is running, the action is **not repeated**: it becomes a wait that says it may already have happened and asks what to do.

## The host

Schedules, events, webhooks, timers and runs that were waiting need something awake. That is **the desktop app while it is open**, or **`zorix serve`** (headless, for a server). Only
one program owns the data folder, so only one host exists.

- A schedule that fires while nothing runs is **skipped and recorded** ("Missed 3 runs while ZORIX was closed"), not replayed: a morning report is not sent at night.
- Webhooks are **off** until you turn them on (Settings › General, or `zorix serve --webhooks`). They listen on `127.0.0.1:7788` unless you give another address.

### Webhooks

`POST /hooks/<workflow>/<secret>`. The secret is made per workflow and kept in the keychain; `zorix workflow webhook NAME` and the editor show the address, and *make a new one*
replaces it. A wrong secret, an unknown workflow and a workflow that is off all answer the same `404`. A body over 1 MB answers `413`. With *signed*, a request needs a valid
`X-Hub-Signature-256` (HMAC-SHA256 of the body with the signing secret, as GitHub sends) or it answers `401`. The body is JSON (anything else arrives as `{"body": "…"}`) and
reaches steps only through templates: it is data, never an instruction: in an agent's prompt every value a template puts in is wrapped as `<data … untrusted="true">`, so what a payload says cannot pose as the workflow's own words. **A repeated delivery is recognised.** When the sender names each delivery (`X-GitHub-Delivery`, `X-Gitlab-Event-UUID`, `Webhook-Id` or `Idempotency-Key`), an id
that was already received in the last 24 hours answers `200 {"duplicate": true}` and starts nothing, whatever its body; a signature is checked first, so an unsigned
request cannot use up an id. A sender that sends no id cannot be told apart from a new delivery (a timer may legitimately post the same body again and again), so each one
starts a run: a signed request captured on the way can be sent again until the signing secret is replaced. To receive from the internet, put a tunnel or a reverse proxy in front: ZORIX does not open itself to it.

## The command line

```
zorix workflow list | show NAME | create NAME | edit NAME | import FILE | export NAME [--version N] [-o FILE]
zorix workflow versions NAME | diff NAME FROM TO | publish NAME [--note TEXT] [--confirm-unattended] | rollback NAME N
zorix workflow enable NAME | disable NAME | remove NAME | templates | add TEMPLATE | webhook NAME [--reset]
zorix workflow run NAME [--input JSON] [--test] [--trigger STEP]     follows the run; exit 3 when it waits for a person
zorix workflow runs [NAME] | run-show RUN | cancel RUN | waits | answer WAIT --approve | --refuse | --text "…"
zorix serve [--webhooks] [--bind ADDRESS]
```

Managing needs no engine, so it works while the desktop app is open. `run`, `answer` and `cancel` need the engine and so the data folder: close the app, or use the app or
`zorix serve` for them. A workflow is a file, `<name>.workflow.json` (`"zorix": "workflow/1"`), strict about unknown keys and written the same way every time. Three starters add
as drafts that are off: `daily-digest`, `incident-triage` and `weekly-report`.

## In the app

*Workflows* in the sidebar. The **list** shows what starts each workflow, its last 14 runs (a bar per run, as tall as it was long: green finished, amber waited for a person, red
failed), its version and state, and an On switch. The **editor** has a palette (click or drag to add), a canvas (drag a step to move it, drag from its bottom dot to another step to join
them, drag the background to pan, Ctrl/⌘ and scroll to zoom, Delete, arrow keys and undo; the steps are also listed as text for screen readers), an inspector per kind, *Versions*,
*Runs*, *Test run* and *Publish*. The **run view** is the same canvas read-only with the state of every step, and an approval, a question or an action card for the step that waits.
The design system's *Workflow run* screen was specified for this phase in `design/` ([what was added](../design/local-additions.md)); [known differences](design-system.md#known-differences) are listed.

## Limits

An `agents.delegate` tree is open-ended; a workflow is fixed on purpose. Equal budget shares per loop item, one definition per name, no sub-workflows, one host at a time,
and a missed schedule is never replayed.
