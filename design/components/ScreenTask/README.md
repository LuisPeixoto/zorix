# Task

One task, in a Drawer beside the board.

- Built from Drawer, Stepper, KeyValue, Callout, Chip, Badge, Avatar, Sparkbar, List and Textarea.
- Header: the key in mono, the priority and the title. Below, a **Stepper** of the four states (Backlog, In progress, Review, Done), with the current one marked.
- A `warn` Callout says what is needed from you when the task is in Review or its run failed or waits for a person.
- Fields: assignee (a person or an agent), priority, dates, progress, what it depends on (each with its state) and its links. A pull request link shows its state when a connection can read it.
- **Run** shows the current run with its state, time and cost, and *Open run*. **Deliverables** lists what it produced. **Activity** lists what happened, newest first.
- Footer: *Ask Atlas* (when the project has a manager), *Send back* (asks for a note) and *Mark done*. Moving to In progress is the Stepper or a drag; for an agent it starts a run.
- When a manager asks to move a task to Done, an ApprovalCard shows the exact change with a fingerprint, like any other approval.
