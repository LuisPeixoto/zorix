# ADR-0049: Files attached to a run

Status: accepted (Phase 11)

## Context

A run could be given a prompt and, from code, a block of JSON. A person who wanted a run to read a contract or a log had to open the file, copy its text into
the prompt and lose its name, its pages and every limit. The design system draws the way out: an `@` in the prompt that picks a document, files dropped on
the form, chips for what is attached, and the size and the expected cost before the run starts. The app had none of it.

## Decision

- **A file is data the person chose.** `TaskRequest.attachments` holds, for each file, its **name** (never its folder), the **text** read from it, its size
  and whether the text was cut. The run is given the text in a required segment right after the task, wrapped as an untrusted `<document source="attachment:NAME">`
  (the preamble already tells the model that a `<document>` is data and to cite its `source`), marked untrusted, with the name and the text defused so that neither can close the
  wrapper or an attribute. The person's own prompt never contains a word of the file. An attachment adds no permission: it is not a grant, and an
  agent's run keeps its own.
- **A choice is a permission to read that file, and only that file.** A file outside the workspace folder is allowed because a person picked it, and the form
  says so; the run still cannot open another file from there. Nothing a file says changes what a run may do (a test makes the model obey a hostile file and shows the call denied).
- **Three ways in, one rule.** The button (a file dialog), `@` in the prompt (a menu of indexed documents and workspace files, typed to filter, arrows and Enter) and dropping files on
  the window, in New run and in the chat with an agent; `zorix run --attach FILE` (repeatable) on the command line. Each way gives paths; the program reads them. The interface
  checks a file when it is chosen (nothing is read) so a refusal shows at once; starting the run checks and reads again, and any refusal stops the run before it exists.
- **What is checked, before anything is read:** it is a regular file (not a folder, a pipe or a device); neither its name nor its real name (after links) is one that holds
  secrets (`.env*`, keys, `credentials*`, `id_*` …); it is not below a folder of credentials (`.ssh`, `.aws`, `.gnupg`, `.kube`, `.docker`, `.azure`, `.config/gcloud`); it is at most 1 MB of
  text or a 25 MB PDF; there are at most ten files, three million characters together, and the same file twice counts once. A file ZORIX does not know by extension is read as
  text when it is text; a binary one is refused.
- **Read like a document, then cleaned.** The same extractors as the indexed documents (text, code, Markdown, HTML as text, PDF by page with `[page N]` kept so an answer can cite
  it, and a scanned PDF through [OCR](0048-watching-folders-and-reading-scans.md) when reading scans is on). Anything that looks like a secret is removed before the text is stored. The
  text of a file is cut at one million characters, and when the files together are more than half of the model's window they are cut in proportion, with a sentence inside each
  document saying so, instead of making the run fail.
- **Every way of making a run has the same limits.** `NewExecution::validate` checks the number, the names (up to 255 characters, no folders) and the text, so a client of the engine
  that is not the desktop or the command line cannot get around them.
- **What is kept, and for how long.** The text is stored with the run (it is part of what was asked, and a resume needs the same bytes). Retention in *compact* mode drops the text and keeps the
  name, the size and the fact that the file was given; *delete* removes the run. The estimate before the run counts the files (a text file by its size, a PDF by an eighth of it).

### Security review

| Threat | What stops it |
|---|---|
| A file that holds secrets is attached | Names are checked as typed and as they are after links (a link called `notes.txt` that points at `.env` is refused), and so is the folder: nothing below a credentials folder is read. Secret-looking text is removed from what is kept. |
| A pipe or device makes the read wait for ever | Only a regular file is opened. (The check and the open are two steps, so a file swapped for a pipe between them is not caught; the limit is stated, as for the file tools.) |
| A huge file, or one that grows while it is read | Sizes are checked first and the read itself stops one byte past the limit; a PDF has its own time, size and page limits. |
| Text in the file tries to give orders, close its wrapper or pose as the person | Wrapped as untrusted data after the task, marked `untrusted`, closing tags defused in any spelling, the name escaped inside its attribute; it is never part of the person's message. Tests: the wrapper, a hostile name, and a whole run in which the model obeys the file and the call is denied. |
| An attachment widens a run's permissions | It is data in the request, not a grant; an agent's run is built with the same grants and limits with or without files (tested). |
| A client sends more than the interface allows | `NewExecution::validate` refuses more than ten files, a name with a folder in it, and too much text, for every caller. |
| The text lives in the database for ever | Compaction drops it; the setting that keeps runs for a number of days applies. It is stated in the persistence guide. |
| The `@` menu lists files a person should not offer | It lists only what the indexer would read (the walk leaves out hidden and ignored files and private names) from the person's own indexed folders and the workspace they named. |

## Consequences

- A run can be asked about a document without indexing it, and the answer can cite its page.
- A run that was given files is bigger: the files take up to half the model's window, and the estimate says so.
- A file that is attached is the file as it was when the run started; changing it later does not change the run.
- Scanned PDFs are read only when reading scans is on; otherwise the refusal says how to turn it on.
