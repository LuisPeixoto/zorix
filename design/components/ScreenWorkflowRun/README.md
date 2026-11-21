# Workflow run

One run of a workflow: the same canvas as the editor, read-only, with the state of each step, and the selected step's rail.

- Built from PageHeader-style sub-bar, TriggerChip, WorkflowCanvas, WorkflowNode, ApprovalCard, KeyValue, Card and Badge.
- Header: breadcrumb (Workflows, the workflow, Runs), `Run #N`, a state badge, the version that ran, what started it, when, and the cost so far. Actions: Open in editor (ghost) and Cancel run (secondary).
- Steps show their state on the node: `done` (with the time or the result), `running`, `waiting` (a person must decide; the warn halo), `failed`, and `skipped` (the branch a condition did not take, drawn with dotted edges).
- The rail shows the selected step. A waiting **approval** step shows an ApprovalCard with what the step chose to show and a fingerprint of it; the answer is bound to that fingerprint. A **question** step shows the question, a text field and Send. An agent or team step shows its summary and **Open run**, which opens that execution's work tree.
- Below the card, "Steps of this run" lists every step in order with its state, so the run can be read without the canvas.
- A run started from **Test run** says so in the header ("Test run") and asks before every action.
- A waiting step appears in the Inbox with the same card.
