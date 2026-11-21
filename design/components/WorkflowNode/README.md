# WorkflowNode

One step of a workflow. Its kind sets its colour: trigger `node-trigger`, agent `node-agent`, logic `node-logic`, approval `node-approval`, action `node-action`.

- Anatomy: a 2px kind-coloured top edge, an icon tile tinted with the kind, title, kind and detail line, in and out ports.
- States: idle, selected (accent border and `ring`), running (`info` border), waiting for a person (`warn` halo), failed (`err` border and badge).
- Titles say what the step does ("Post to #incidents"), not the tool's name.
