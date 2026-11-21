# Badge

A word or two of state, in a tone: ok, warn, err, info, muted, accent.

- Variants: `soft` (default), `outline`, `solid`. Options: `dot`, `live` (a pulsing ring for something happening now).
- A badge always has a word. Colour alone never carries the state.
- Map states to tones in one place (`stateTone` in `format.ts`): running → info, completed → ok, waiting for approval → warn, failed → err, cancelled → muted.
