# Project timeline

The project's tasks across weeks: each a bar from start to due date, filled by progress, with who has it and a line for today.

- Built from PageHeader, Tabs, Segmented and the timeline grid (task bars tinted by priority or state).
- Bars keep the word and the percentage; colour is never the only signal.
- **Dependencies are drawn.** An arrow runs from the end of a task to the start of the task that waits for it. A `err` arrow means the later task starts before the earlier one is due.
- Tasks without a start or a due date are not drawn; they are listed below the grid as *Not scheduled*, and giving them dates places them.
