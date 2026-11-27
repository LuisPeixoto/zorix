# ADR-0043: Summaries of long conversations

Status: accepted (Phase 11)

## Context

When the history of a run no longer fit the model's window, the context builder left the oldest turns out and said so in a
short note ([ADR-0007](0007-context-builder.md)). The run then forgot what it had found in those turns, and a long job did the
same work again. The roadmap asked for a summary of what was left out.

## Decision

- **The run loop makes the summary, the builder only shows it.** After the context is built, if turns of the history were left out and
  the setting is on, the loop asks a model to summarise exactly the turns that were left out (and the summary that already stood
  for older ones), keeps the result in the loop's state, and builds the context again. The summary is shown as the first turn of
  the history, ahead of the turns that fit; it is the last thing the builder drops.
- **The transcript is never cut.** The state keeps `summary { covers, text }`, and the messages the summary stands for are still in
  the transcript, in the checkpoint and in the events. A resume reads the summary from the checkpoint, so every resume shows the model
  the same bytes and no second summary of the same turns is asked for ([ADR-0015](0015-frozen-recall-snapshot.md)). The summary is
  always cut at the end of a turn, so a tool call is never separated from its result.
- **A summary is untrusted data.** It was written from tool output. It is wrapped like a tool result (`<data source="conversation.summary"
  untrusted="true">`, with every tag of ZORIX defused in any spelling), its provenance is untrusted, anything that looks like a credential
  is replaced before it is sent to be summarised and again before it is kept, and it is capped at 6,000 characters. The input sent to
  the summariser is bounded too: each piece is cut at 2,000 characters and the whole at 60,000.
- **It costs what a model call costs, and says so.** The call uses the run's model, its usage is added to the run's (so its limits and
  the worker's budget count it), and an `execution.context.summarised` event records how many messages were folded, the size of
  the summary and the cost, never the text. It is not made when the run is already at a limit.
- **It never fails a run.** If the call fails, times out or answers nothing, the turns stay left out with the old note, and no other
  attempt is made until the run is picked up again.
- **It is a setting, on by default** (`context.summaries`). An engine built without settings keeps the old behaviour.

## Consequences

- A long run keeps what it found at the price of one extra model call each time more turns stop fitting.
- Native turns (reasoning state a provider wants replayed unchanged) cannot be replayed after a summary, the same as after dropping a
  turn: the request says its history was edited.
- Only the history of one run is summarised. The conversation exchanges of earlier runs that no longer fit are still left out, newest
  first, as before; summarising those is left open.
- A summary can be wrong. The original turns are in the run's record, and the summary says it may be incomplete.
