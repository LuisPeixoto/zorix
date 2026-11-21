# Field

Label, control, hint and error as one unit. Every control sits inside a Field.

- Field props: `label`, `hint`, `error`, `required`. It passes `id` and `aria-describedby` to the control.
- Input props: `bind:value`, `invalid`, `mono` (keys, paths, ids), `icon`, `trailing`. Textarea and Select take `bind:value` and `invalid`.
- Fields rest on `panel-2` and rise to `panel` with an `accent` border and the soft `ring` on focus.
- Errors say what is wrong and how to fix it, in `err` under the control: "That folder does not exist."
- Hints say where a value goes: "Stored in the system keychain, never in a file."
