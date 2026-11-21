# WorkflowCanvas

The visual editor and live view of a workflow: triggers, agents, conditions, approvals and actions joined by edges.

- The canvas is `panel-2` with a 16px dot grid in `faint`. Nodes snap to that grid.
- Edges are 1.5px `line-strong` curves from an out port to an in port. A step that is running draws its edge as a moving accent dash; finished paths tint toward `ok`, and the path of a branch that was not taken is a faint dotted line (`skip`). Branch labels (yes, no, else) sit on the edge in a pill.
- A floating glass toolbar (add step, zoom, fit, undo) sits bottom-left and a minimap bottom-right.
- The same canvas is the run view: a workflow run shows state on each node, and an approval node opens the ApprovalCard.
- Workflows are versioned like agents; publishing writes a new version and the next trigger uses it.
