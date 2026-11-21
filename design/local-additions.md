# Additions made in this repository

The design system lives in a claude.ai Design System artifact and is vendored here ([ADR-0032](../docs/adr/0032-design-system-v3-is-the-contract.md)).
When a phase needs a screen the design system lacks, it is specified here first, in the same form as the rest of the package, and this file
lists it so the artifact can be brought in line. **Port every entry below back into the artifact, then delete the entry.**

| Added | In | What |
|---|---|---|
| 2026-11-21 | `components/ScreenWorkflowRun/` (README, preview), `screens/workflow-run.png`, `manifest.json`, `surfaces.md` | The **Workflow run** screen: the editor's canvas read-only with the state of each step, the selected step's rail with an ApprovalCard, and a list of the steps |
| 2026-11-21 | `components/bundle.css` | `.zx-node--done`, `.zx-node--failed`, `.zx-node--skipped` and the edge class `path.skip` |
| 2026-11-21 | `components/WorkflowNode/README.md`, `WorkflowCanvas/README.md`, `index.d.ts` | The `skipped` state of a step and the `skip` state of an edge (a branch a condition did not take) |
