# LogStream

Events and logs as they happen: time, level, source and message, one line each.

- `mono` at 11.5px with tabular times. Level words carry the tone (OK, INFO, WAIT, WARN, ERR); the message stays `fg`.
- Secrets are redacted before a line is stored. A line opens the event in a Drawer, with its run and span.
- Live streams can pause; a paused stream says how many lines arrived since.
