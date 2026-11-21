# Additions made in this repository

The design system lives in a claude.ai Design System artifact and is vendored here ([ADR-0032](../docs/adr/0032-design-system-v3-is-the-contract.md)).
When a phase needs a screen the design system lacks, it is specified here first, in the same form as the rest of the package, and this file
lists it so the artifact can be brought in line. **Port every entry below back into the artifact, then delete the entry.**

| Added | In | What |
|---|---|---|
| 2026-11-21 | `components/ScreenWorkflowRun/` (README, preview), `screens/workflow-run.png`, `manifest.json`, `surfaces.md` | The **Workflow run** screen: the editor's canvas read-only with the state of each step, the selected step's rail with an ApprovalCard, and a list of the steps |
| 2026-11-21 | `components/bundle.css` | `.zx-node--done`, `.zx-node--failed`, `.zx-node--skipped` and the edge class `path.skip` |
| 2026-11-21 | `components/WorkflowNode/README.md`, `WorkflowCanvas/README.md`, `index.d.ts` | The `skipped` state of a step and the `skip` state of an edge (a branch a condition did not take) |
| 2026-11-21 | `components/ScreenProjects/`, `ScreenProjectList/`, `ScreenProjectDeliverables/`, `ScreenProjectHistory/`, `ScreenTask/`, `ScreenNewProject/` (README, preview each), `screens/projects.png`, `project-list.png`, `project-deliverables.png`, `project-history.png`, `task.png`, `new-project.png`, `manifest.json`, `surfaces.md` | The Phase 8 screens the design system did not show: the **Projects** index, the project's **List**, **Deliverables** and **History** tabs, the **Task** drawer and the **New project** modal. A manager is asked for through *Ask Atlas*, which sits where the board shows *Share* |
| 2026-11-21 | `components/ScreenProjectTimeline/` (README, preview), `screens/project-timeline.png`, `components/bundle.css` | The timeline draws **dependency arrows** (`.zx-gantt__deps`, an `err` arrow when a task starts before the one it waits for is due) and lists tasks with no dates as **Not scheduled** (`.zx-gantt__note`) |
| 2026-11-21 | `components/ScreenWorkforceList/`, `ScreenWorkforceActivity/`, `ScreenWorker/`, `ScreenHireWorker/`, `ScreenOrgSettings/` (README, preview each), `screens/workforce-list.png`, `workforce-activity.png`, `worker.png`, `hire-worker.png`, `org-settings.png`, `manifest.json`, `surfaces.md` | The Phase 9 screens the design system did not show: the Workforce **List** and **Activity** tabs, the **Worker** page (metric tiles, autonomy with budget, time and the access that always asks, a manager's schedule, reports and latest report), the **Hire a worker** modal and **Org settings**. A manager's figures include everyone who reports to it |
