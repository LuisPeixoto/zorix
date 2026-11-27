# ADR-0045: Estimates before a run

Status: accepted (Phase 11)

## Context

Limits only stop a run once it has gone over them, and nothing said beforehand what a run would probably cost or how long it would take. The design
system draws an *Expected ≈ $0.35 · 3 min* line and Budget, Time and Steps fields in New run; the app had neither. A bad estimate is worse than none:
a number that looks precise and is invented teaches people to ignore it.

## Decision

- **The estimate is what similar runs took.** The median of the last twenty finished top-level runs of the same agent (or of no agent) on the same model:
  tokens sent, tokens written, model calls and work time. A run that was compacted still counts, because it keeps its numbers. When the agent's model is
  its own choice, the runs on any model count, and the cost is what they cost.
- **With fewer than three such runs it says it is rough**, and uses fixed guesses for a run of three model calls (350 tokens of fixed text, the prompt,
  350 tokens written and six seconds a call). Files attached to the run add their tokens to every call.
- **A cost is never invented.** The price of the model for the estimated tokens, from the catalogue; `Some(0)` for a model on this computer; unknown
  for a remote model with no price, shown as *cost unknown*, never as $0.
- **It is a guess, not a limit.** The limits are separate fields of the run (budget in dollars, minutes of work, steps); an agent's can only be
  narrowed. A budget on a model with no known price is refused before the run starts ([providers](../providers.md#models-and-numbers)).
- **One implementation, three faces.** `zorix_core::estimate_run` is a pure function of the samples; `zorix_hub::estimate` reads the samples from the store
  and the price from the router; the command line (`zorix run --estimate`) and the desktop app (the `estimate_run` command, shown in New run) call it.
  Estimating starts nothing and locks nothing.

## Consequences

- A person with no history sees "a rough guess" and is not misled; the number improves as they use the app.
- The estimate does not know what a prompt will make the run do, only what runs like it did. A prompt that is nothing like the others is
  mispredicted, and the interface says it is a guess.
- Samples are read from run records with `json_extract`; a run with no usage recorded counts as zero tokens, which is why a minimum of three
  is required before the history is trusted.
