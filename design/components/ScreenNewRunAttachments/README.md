# New run · attachments

Files a run is given up front, by `@` or by dropping them.

- Built from Textarea, Chip, the CommandPalette's list as a popover, Callout, KeyValue and Button.
- Typing `@` in the prompt opens a menu of indexed documents and workspace files; choosing one adds it as a chip in the sentence and under **Attached**. Dropping files on the form attaches them the same way. At most ten files; each is checked for size and for private names before the run starts, with the reason when one is refused.
- A file is given to the run as data, wrapped like every document; nothing in it is an instruction, and attaching it gives the run no permission to read any other file. A file outside the workspace folder is allowed because a person chose it, and the form says so.
- **Before you start** lists the attached files and the expected cost and time, from the last twenty similar runs. The estimate is a guess, not a limit; the budget field is the limit.
