# Modal

A dialog that asks one question and blocks the page until it is answered.

- Props: `bind:open`, `title`, `icon`, `size`; snippet `footer`. Traps focus, closes on Escape and on the backdrop, gives focus back.
- The title is the question. The primary button repeats the verb ("Delete agent"), never "OK".
- Opens with a 200ms fade and scale on `ease-out`.
