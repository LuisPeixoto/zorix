# Button

The one way to start an action: primary, secondary, ghost, danger and subtle, in three sizes.

- **One primary per screen**, for the thing the screen exists to do (Start run, Approve, Publish workflow). Everything else is secondary or ghost.
- `danger` for actions that remove or refuse (Remove key, Refuse). Ask for confirmation in a Modal when the action cannot be undone.
- `subtle` (accent tint) for a suggestion the app makes, such as Suggest or Add from templates.
- Props: `variant`, `size` sm · md · lg, `icon`, `iconRight`, `loading` (keeps its width, shows a spinner), `disabled`.
- Labels are verbs that say what happens: "Start run", not "OK". Sentence case.
