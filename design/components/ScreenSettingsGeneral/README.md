# Settings · general

What the program keeps and where it reports, beside the data folder and webhooks.

- Built from the Settings tabs (General, Appearance, About), Card, ToggleRow, Field, Input, Segmented, Callout and KeyValue.
- **Long conversations** is one switch, on by default: the oldest turns are summarised instead of dropped, at the cost of one extra model call each time.
- **Keep runs** says how many days to keep finished runs (1 to 3650; empty keeps them for ever) and what happens to an older one: *Compact* keeps the request, the answer, the cost and the time and drops the events, the steps and the recall; *Delete* removes the run. The callout says how many runs are older than that and how much room compacting frees, before anything happens. *Compact now* is secondary and asks to confirm; Save is the only primary.
- **Telemetry** is off until an address is given (https, or http for a collector on this computer). The warning says what leaves the computer and what never does. The header a collector needs is kept in the system keychain, never in the database.
