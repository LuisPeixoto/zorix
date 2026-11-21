# Projects

A project holds **tasks** for you and your agents. A task given to an agent or a team makes it work: moving the task to *In progress* starts an ordinary run, and when the run ends the task
goes to *Review* with the answer as a deliverable. Decisions: [ADR-0036](adr/0036-projects-and-tasks-are-records-that-create-runs.md) (records that create runs) and
[ADR-0037](adr/0037-project-tools-and-the-manager.md) (the tools of a manager).

A task adds no new way to run a model. Its run is an **ordinary run**, with its tree, approvals, cost and events, linked back by the metadata `project` and `task`. The run view says
*From task OPS-38* with a link.

## The shape

- A **project** has a **key** (2 to 5 capitals or digits, starting with a letter: `OPS`), a name, a description, an optional **manager** (an agent), optional start and due dates, and an optional
  **cost limit for each task run**. The key never changes and is never reused. A project can be archived, which hides it and keeps everything.
- A **task** has a key from its project (`OPS-38`, numbers are never reused, not even after a delete), a title, a description, a **lane**, a priority, who has it, progress, dates, what it waits for,
  links, deliverables, its runs and its history.
- **Lanes** are fixed: *Backlog*, *In progress*, *Review*, *Done*. The rules below depend on what each one means.
- **Priority** is urgent, high, normal or low, shown as a flag, a word and a tone.
- **Who has it** is *you*, an agent, a team (through its lead), or nobody. There are no accounts until Phase 10, so *you* is the only person and there is no *Share*.
- **Dates** are days with no time zone. A late due date turns red and says *Today* or *2 days late*.

## What moves a task

| Move | What happens |
|---|---|
| To **In progress**, a task an agent or a team has | A run starts, with a prompt made from the task, its project and what it builds on. It may spend at most the project's limit for a task run (on top of the agent's own limits) |
| To **In progress**, a task you have or nobody has | Nothing starts; it is a card in a lane |
| A run **finishes** | The task goes to **Review**, its progress is 100 and the final answer becomes a deliverable. It *needs you* |
| A run **fails** | The task stays in *In progress*, marked *Run failed* with the reason. It needs you. *Run again* starts a new run, with an optional note |
| To **Review**, from the drawer | It needs you until you act |
| **Send back** (Review to *In progress*) with a note | The agent runs again, and the note is in its prompt |
| To **Done** | Progress is 100. A task under a manager asks like any other change (see below) |
| Out of *In progress* while a run is going | ZORIX asks first, then stops the run. What the run already produced stays |
| To **In progress** while a task it **waits for** is not Done | The card says *Waiting for OPS-31*; the run starts by itself once it is Done |

A task has **dependencies** on tasks of its own project, with no cycles. The timeline draws them. *Needs you* is never stored: it is read from the task (in Review, a failed run, a run waiting for an
approval) and it also puts an item in the **Inbox**.

## The manager

A project can have a **manager**: an agent that plans it. *Ask the manager* starts a run of that agent with access to this one project, and while it is going the project says it is planning. It
works through tools:

| Tool | What it does | Asks? |
|---|---|---|
| `tasks.list`, `tasks.get` | Read the tasks of the project | No |
| `tasks.create`, `tasks.update`, `tasks.comment` | Plan and write | Yes, every call |
| `tasks.move` | Move a task, a move to Done included | Yes, and only a person looking at it can approve: no automatic mode can |

The grant is `project:KEY` (read) or `project:KEY:write` (plan). A manager cannot touch another project. History records "asked to move OPS-33 to Done, approved by you".

A run a task started gets `task:KEY` and nothing wider: `task.report_progress`, `task.add_deliverable` and `task.note` on that one task, which only change the task's own record and so do not ask.
This is where real progress (64%) and an agent's own deliverables come from.

## Views

*Projects* sits in the Work group of the sidebar.

- **Projects** lists them as cards: key, name, progress ("18 of 29 tasks"), the manager, the due date, who has tasks, and how many need you. *Active* and *Archived*.
- **Board** shows the four lanes. Drag a card with the pointer (it tilts and lifts), or with the keyboard: Space picks it up, the arrows move it between lanes and places, Space drops it and Escape
  lets it go; each step is said in a live region. *Add task* is at the foot of each lane. Filters: *Everyone*, *Agents*, *People* (the tasks you have) and *Urgent and high*.
- **List** is the same tasks as a table you sort by key, priority, state, progress or due date.
- **Timeline** draws each dated task as a bar from its start to its due date, filled by progress and tinted by priority or state, with a line for today, in *Days*, *Weeks* or *Months*. An arrow
  joins a task to the one that waits for it, and is red when the later one starts before the first is due. Tasks with no dates are listed under the grid.
- **Deliverables** lists what the project produced, by task: an agent's answer (it opens in place), a link, a file's text.
- **History** is everything that happened, newest first, in sentences, with filters for moves, runs and approvals.
- The **task drawer** edits a task in place (fields, dependencies, links), shows the run with its cost and *Open run*, the deliverables, the activity and a note field, and has the buttons that
  move it. A link to a pull request shows its number, and its state (open, merged, draft) when a GitHub or GitLab connection can read it.

Search (⌘K) finds projects and tasks by key and title. The design system's [known differences](design-system.md#known-differences) are listed, and the screens this phase added to it are
logged in `design/local-additions.md`.

## The command line

```
zorix project list [--archived] | create KEY NAME [--manager A] [--due D] [--cost-limit USD] | show KEY | edit KEY … | archive KEY | restore KEY | remove KEY
zorix project ask KEY [PROMPT]       asks the manager to plan, and follows its run
zorix task list [KEY] [--state S] [--mine] [--needs-you]
zorix task add KEY TITLE [--assign WHO] [--priority P] [--start D] [--due D] [--after OPS-31] [--state S]
zorix task show T [--full] | edit T … | comment T TEXT | link T URL | deliver T (--text | --file | --url) | remove T | history KEY|T
zorix task move T STATE [--note TEXT] [--stop-run] [--no-wait]    follows the run; exit 3 when it waits for a person
zorix task restart T [--note TEXT]
```

Managing works while the desktop app is open, because it needs no engine. Moving a task to *In progress*, `task restart` and `project ask` start a run, so they need the engine and with it the
data folder: close the app, or do it there. The app and `zorix serve` keep tasks moving on their own (a finished run sends its task to Review; a task that waited for another starts).

## Persistence

Schema 7: `projects` (with the counter that numbers tasks), `tasks` (with a revision for optimistic writes) and `project_events` (the history). See [persistence](persistence.md).

## Limits

No custom lanes, recurring tasks or import and export of a project; no workflow step or trigger for tasks; no *Share* or other people (Phase 10); no weekly planning or autonomy levels (Phase 9); a
task does not record when it finished.
