ZORIX is a harness for AI: it decides what a model may see and do, stops to ask a person before anything changes, and records every step. The interface exists to make that control visible and calm. This system covers the whole product: runs and approvals, providers, memory, integrations, agents and teams, and the workflows, projects, AI workforce, Command Center and Marketplace that complete it.

Tokens marked **v3** in their usage note were added for workflows, projects, the workforce, the Command Center and the Marketplace. Every other value is copied from `apps/desktop/src/app.css`.

## Principles

- **Calm surfaces, one accent.** Surfaces stack from the window to dialogs: `bg` → `panel-2` → `panel` → `elevated`, separated by 1px `line`. The violet `accent` is a tint (`accent-soft`, text in `accent`) for what is selected or active, and a solid fill only for the one main action of a screen.
- **Colour is never the only signal.** A state always has a word, and an icon or a dot, as well as a tone.
- **The person decides.** Anything that changes something waits for a human. Give approvals the warmest, most visible treatment on any screen (`warn` halo, ApprovalCard), and never hide the exact arguments.
- **Show the record.** Ids, hashes, models, costs and timings are part of the interface, set in `mono` with tabular figures. They are selectable.
- **A desktop app, not a web page.** Compact 13px body, `user-select: none` except where people copy, slim scrollbars, real focus rings, keyboard first (⌘K everywhere).
- **The interface decides nothing.** Components show state; rules live in the Rust core.

## Voice and copy

- Plain words, short sentences, active voice. Write from the person's side: "Notify me when a run needs approval", not "Enable approval webhook events".
- Sentence case everywhere: buttons, titles, menus. ZORIX is always in capitals.
- Buttons are verbs that say what happens: **Start run**, **Approve**, **Publish workflow**, **Delete agent**. Never "OK" or "Submit".
- Say what happened in the past tense: "Key saved", "Workflow published".
- Errors state the fact, then the fix, with no apology: "The provider answered 401: the key was refused. Replace it in Providers."
- Name the consequence before the action: "Changes things, so it asks first." "A cloud provider receives the conversation of a run."
- "You" is the person; agents are named by their name ("researcher asked data-analyst"). Agents never speak for ZORIX.
- No emoji in interface copy. Agent avatars may carry an emoji or an icon chosen by their author.
- Numbers keep their units and real precision: `12 408` tokens (thin space groups), `8.4 s`, `$0.42`, `98.6%`.

## Colour

- Window `bg`; cards, toolbar and focused inputs `panel`; sidebar, card headers, wells and resting inputs `panel-2`; dialogs, menus, toasts, drawers and the palette `elevated`. Hovered rows `hover`.
- Frame cards and fields with `line`; divide rows inside a card with `line-soft`; use `line-strong` for hovered borders and canvas edges.
- Text: `fg` for everything people read; `muted` for secondary text; `faint` only for icons, placeholders and decoration, never body text.
- `accent` for selection, links, focus and the single primary button (with `accent-fg` on it). `accent-2` (cyan) is the far end of the brand gradient and the colour of workflow triggers. It is never a second primary.
- Status: `ok` done, `warn` waiting for a person, `err` failed or refused, `info` running. Tints for badges and callouts are the tone at 8–15% over `panel`.
- Charts use `series-1` to `series-5` in order: the subject, the comparison (dashed), then three more. Workflow node kinds use `node-*`; task priorities use `priority-*`.
- The brand gradient (135°, `accent` → blue-violet → `accent-2`) appears only on the logo tile, empty-state tiles, progress fills and Marketplace art. The primary button has its own gentle top-to-bottom accent gradient.
- Glass (`glass` and `glass-line` with an 18px blur) is for the toolbar, the command palette and floating canvas tools only. It falls back to `panel`.

## Type

- Inter Variable for all text; JetBrains Mono Variable for ids, paths, keys, model names, SQL, logs and figures in rows. Both are bundled (see Fonts); ligatures are off in mono.
- The scale is compact: `body` 13 for text, `control` 12 for small controls, `caption` 11.5 for metadata, `label` 10.5 uppercase with 0.05em tracking for group labels and table headers.
- Titles: `lead` 15 for card titles that lead a screen, `dialog-title` 17, `section` 20, `page-title` 24, `display` 28 for the one hero figure of a Command Center board. Titles are semibold with -0.015em tracking and balanced wrapping.
- Use `tnum` (tabular figures) for every count, time, size and cost that can change.

## Space, shape and depth

- A 4px grid: `space-1` 4 inside chips, `space-2` 8 between controls, `space-3` 12 for row padding, `space-4` 16 for card padding and gaps between cards, `space-6` 24 for the page gutter, `space-8` 32 between sections.
- Radius by role: `radius-lg` 8 for buttons, fields and the logo tile; `radius-xl` 12 for callouts, menus, workflow nodes and task cards; `radius-2xl` 16 for cards and dialogs; `radius-full` for chips, badges and dots.
- Depth in three steps, each a contact shadow and an ambient one: `sh-1` rows and secondary buttons, `sh-2` cards that float and nodes, `sh-3` sheets and dialogs. `sh-card` is the resting card, `sh-pop` the toast, `sh-raised` adds light on the top edge of raised tiles.
- Focus: a 2px `accent` outline offset 2px on every interactive element; fields add the soft `ring`.

## Motion

- 120 ms for hover and press, 150 ms for controls, 200 ms for dialogs, drawers and toasts. Nothing takes longer.
- `ease-out` by default; `ease-spring` for things that appear (a toast, a new node, a new task).
- Motion means something is happening now: a live dot pulses, a running workflow edge moves, a streaming answer shows a caret. Nothing loops for decoration.
- Everything stops under `prefers-reduced-motion`.

## Iconography

- One icon set: Lucide-style paths on a 24px grid with a round 1.8px stroke, stored as static markup in `icons.ts` (see the Icon card). v3 adds twenty icons for the final surfaces (workflow, calendar, webhook, zap, split, flag, kanban, target, building, gauge, activity, store, badge-check, star, zoom-in, maximize, paperclip, at, dollar, filter).
- Icons take `currentColor`: `muted` at rest, `accent` when active, a status tone only beside a status word.
- Sizes: 13 in badges, 14 in small buttons, 15 in buttons and rows, 20 in tiles, 28 in empty states.
- Third-party services show their Simple Icons mark (CC0) in `fg` beside their name (BrandLogo). A service without a mark gets `plug`.

## Layout

- The shell: a glass toolbar with the mark, the ⌘K search and the theme control; a `panel-2` sidebar grouped by Work (Runs, Workflows, Projects), Organisation (Agents, Teams, Workforce), Connect (Providers, Integrations, Memory, Documents) and Operate (Command Center, Marketplace, Settings); and the page on `bg` with a faint `page-wash` at the top.
- Pages open with a PageHeader (title, one-sentence subtitle, at most one primary action), then content in Cards on a 16px gap.
- Lists of things use List; one thing beside its list opens in a Drawer; a question that blocks uses a Modal.
- Canvases (workflows, org charts) and boards (projects) take the full page width; everything else keeps running text near 62 characters.

## Accessibility

- Text tokens pass WCAG AA on every surface in both themes, and a test enforces it: `fg` ≥ 7:1, `muted`, `accent` and every status ≥ 4.5:1, `faint` ≥ 3:1, text on a solid accent ≥ 4.5:1.
- Every icon-only button has a label. Every control is inside a Field. Status always has a word.
- Dialogs and drawers trap focus and give it back; Escape closes them. Toasts are polite, errors assertive.
- Scrollable regions (code, diffs, logs, canvases) take focus so the keyboard can scroll them. Trees, boards and canvases move with the arrow keys.
- Charts have a text alternative with the headline figure. Glass never carries text that would fail on its solid fallback.

## Do and don't

- **Do** keep one solid accent per screen, for the thing the screen exists to do.
- **Do** show the exact call, the cost and the model wherever an agent acts.
- **Do** make people and agents look different: round initials for people, square avatars for agents.
- **Don't** invent colours or sizes in a screen; add a token or use a scale step.
- **Don't** use `faint` for text people must read, or a tone without a word.
- **Don't** add gradients, glows or glass beyond the places listed above.
