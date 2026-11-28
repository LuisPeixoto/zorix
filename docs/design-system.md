# Design system

ZORIX has one design system, and the app follows it strictly. The system is a package, vendored in [`design/`](../design/README.md): the
**tokens** (`design/tokens.json`), a **specification for every component** (`design/components/<Name>/README.md` with a live
preview and `index.d.ts`), the **surfaces** of the product (`design/surfaces.md`) and **18 assembled screens** at 1440 px
(`design/screens/*.png`). The app implements it: the kit lives in `apps/desktop/src/lib/ui/`, the tokens in
`apps/desktop/src/app.css` (generated from `design/tokens.json`), and both are shown live on the **Design system** screen
(<kbd>Ctrl</kbd>/<kbd>⌘</kbd> + <kbd>⇧</kbd> + <kbd>D</kbd>, or the sidebar item in development builds).
[ADR-0032](adr/0032-design-system-v3-is-the-contract.md) is the rule; this page is how to follow it. Light is the default theme.

| Light | Dark |
|---|---|
| ![The design system in the light theme](images/ds-light.png) | ![The design system in the dark theme](images/ds-dark.png) |

## The contract

1. **Look first.** Before building or changing a screen, open its image in `design/screens/` and the README of every component it
   uses (`design/surfaces.md` lists them).
2. **Tokens only.** Colours, sizes, radii, shadows and durations come from the tokens. `node tools/check-design.mjs` fails on a colour
   literal, an arbitrary Tailwind value for those, a duration over 200 ms, or an emoji in interface copy. An exception goes in the
   allowlist at the top of that script, with its reason.
3. **The kit, as specified.** Use the components of `src/lib/ui/` and give them the props their README names. A component the design
   system has and the kit lacks is built from its README.
4. **Missing in the design system? Add it there first.** Update the design system, re-vendor `design/` (see below), then build.
5. **Missing in ZORIX? Leave it out.** A screen element that needs data or a feature ZORIX does not have yet is not mocked. The
   [roadmap](roadmap.md) lists it under the phase that brings it.
6. **Done means checked.** Light and Dark screenshots of the real app at 1440 px beside the matching `design/screens` image, a design
   critique and an accessibility review with their findings fixed, and every remaining difference written down under
   [Known differences](#known-differences).

## Principles

- **Calm surfaces, one accent.** Surfaces stack from the window to dialogs (`bg` → `panel-2` → `panel` → `elevated`),
  separated by 1px lines. The violet accent is a *tint with accent text* for what is selected or active, and a *solid fill*
  only for the one main action of a screen.
- **Colour is never the only signal.** A state has a word and an icon or dot as well as a colour.
- **Readable by rule.** Text tokens must pass WCAG AA on every surface in both themes; a test enforces it.
- **A desktop app, not a page.** Compact type (13px body), `user-select: none` by default, slim scrollbars, real focus rings.
- **The interface decides nothing.** Components show state; rules live in Rust (see [desktop](desktop.md)).

## Tokens

Tokens are CSS variables chosen by `data-theme="light" | "dark"` on `<html>` and mapped to Tailwind, so classes read like
`bg-panel`, `text-muted`, `border-line`, `bg-accent-soft`. Opacity modifiers work (`bg-err/10`).

| Token | Light | Dark | Use |
|---|---|---|---|
| `bg` | `#f5f6fa` | `#14161d` | The window |
| `panel` | `#ffffff` | `#1b1e27` | Cards, toolbar, inputs when focused |
| `panel-2` | `#f9fafc` | `#20242f` | Sidebar, headers of cards, wells, input background |
| `elevated` | `#ffffff` | `#262a36` | Dialogs, menus, toasts |
| `hover` | `#eff1f6` | `#2a2f3c` | Hovered rows and buttons |
| `line` / `line-strong` | `#e5e8f0` / `#d4d9e4` | `#2f3442` / `#3c4253` | Borders, dividers |
| `fg` | `#121621` | `#eceef4` | Text |
| `muted` | `#566074` | `#a5adbf` | Secondary text |
| `faint` | `#7d879b` | `#7f889e` | Hints and icons, never body text |
| `accent` / `accent-2` | `#5b45f5` / `#0891b2` | `#9d87ff` / `#2dd4ee` | Brand, selection, primary actions |
| `accent-fg` | `#ffffff` | `#14161d` | Text on a solid accent |
| `accent-soft` / `select` | accent at 10% / 14% | accent at 16% / 24% | Tints for active and selected |
| `ok` `warn` `err` `info` | `#047857` `#b45309` `#d12020` `#2563eb` | `#34d399` `#fbbf24` `#f87171` `#60a5fa` | Status |

**Type.** Inter Variable for text and JetBrains Mono for ids, paths, keys and numbers (both bundled, see
`apps/desktop/THIRD-PARTY.md`). The scale replaces Tailwind's: `2xs` 10.5 · `xs` 11.5 · `sm` 12 · `base` 13 (body) · `md` 13.5 ·
`lg` 15 · `xl` 17 · `2xl` 20 · `3xl` 24 · `4xl` 28. Use `tabular-nums` for figures that change.

**Shape and depth.** `rounded-lg` for buttons and fields, `rounded-xl` for callouts and menus, `rounded-2xl` for cards and
dialogs, `rounded-full` for chips and dots. `shadow-card` for cards, `shadow-pop` for what floats.

**Motion.** 120–200 ms, `--ease-out`. Fade and scale for dialogs, fly for toasts. Everything is disabled by
`prefers-reduced-motion`.

**Utilities.** `.brand-gradient` (the logo tile, empty states), `.glow` (the haze behind a page header), `.live-dot` (a pulsing
ring for something happening now), `.skeleton` (loading), `.mono` (monospace without ligatures), `.selectable` (text people
copy: results, errors, ids).

### Tokens added in version 2

Surfaces gained depth and the motion got a language, without changing a single colour the contrast test already covers.

| Token | Use |
|---|---|
| `line-soft` | The hairline between rows inside a card (softer than `line`, which frames the card) |
| `inset` | Light that falls on the top edge of a raised surface (`.raised`, `shadow-raised`) |
| `shadow-1` · `shadow-2` · `shadow-3` | Three elevation steps, each two layers (a contact shadow and a soft ambient one): rows, cards and menus, sheets and dialogs |
| `glass`, `glass-line` | The toolbar and the command palette only: a translucent panel with a blur, and a solid `panel` where the platform has no blur |
| `accent-grad` | The primary button's gentle top-to-bottom gradient. It is kept light enough that `accent-fg` still passes AA |
| `ring` | The soft focus ring of fields |
| `page-wash` | A faint accent glow at the top of a screen |
| `ease-spring` · `.tnum` | A slightly springy easing for things that appear, and tabular figures for counts, times and sizes |

Syntax colours (`.tok-kw`, `.tok-str`, `.tok-num`, `.tok-com`, `.tok-fn`) reuse accent, ok, warn, faint and info, so code in
`CodeBlock` is as readable as any other text.

### Tokens added in version 3

Generated into `app.css` from `design/tokens.json`. They are used by the phases that need them (workflows, projects, the workforce,
the Command Center) and are available now.

| Token | Tailwind | Use |
|---|---|---|
| `series-1` … `series-5` | `text-series-1`, `bg-series-1`, `stroke-series-1` | Chart series in order: the subject, the comparison (dashed), then three more |
| `node-trigger` · `node-agent` · `node-logic` · `node-approval` · `node-action` | `text-node-agent`, … | The kind of a workflow step |
| `priority-urgent` · `priority-high` · `priority-normal` · `priority-low` | `text-priority-high`, … | A task's priority |
| `duration-fast` · `duration-base` · `duration-slow` | `duration-fast` … (120 · 150 · 200 ms) | Hover and press · controls · dialogs, drawers and toasts. Nothing is slower |
| `radius-sm` … `radius-full` | `rounded-sm` … `rounded-full` | 4 · 6 · 8 · 12 · 16 · full. Cards and dialogs are `rounded-2xl` |

**Names.** The design system names text styles by role and the app keeps Tailwind's class names for them:
`display` = `text-4xl` · `page-title` = `text-3xl` · `section` = `text-2xl` · `dialog-title` = `text-xl` · `lead` = `text-lg` ·
`emphasis` = `text-md` · `body` = `text-base` · `control` = `text-sm` · `caption` = `text-xs` · `label` = `text-2xs` uppercase.
`sh-1` / `sh-2` / `sh-3` / `sh-card` / `sh-pop` / `sh-raised` are `shadow-1` … `shadow-raised`.

## Themes

Light is the default. The toolbar button cycles light → dark → system, and the choice is remembered (`localStorage`, key
`zorix.appearance`). `src/early.ts` sets the theme before the first paint, and the native window background follows it, so
there is no flash. In a development build, `VITE_START_THEME=dark` forces a theme without saving it.

## Components

Import from `src/lib/ui/index.ts`. All accept `class`; interactive ones have hover, pressed, focus-visible, disabled
(and loading) states.

| Component | Props and notes |
|---|---|
| `Button` | `variant` primary · secondary · ghost · danger · subtle; `size` sm · md · lg; `icon`, `iconRight`, `loading` (keeps its width). One primary per screen |
| `IconButton` | `icon`, `label` (required: screen reader name and tooltip), `variant`, `size` |
| `Field` | `label`, `hint`, `error`, `required`. Passes `id` and `describedby` to the control through its snippet |
| `Input` · `Textarea` · `Select` | `bind:value`, `invalid`, `mono`, `icon`, `trailing` (Input) |
| `Checkbox` · `Switch` | `bind:checked`, label as children, `description` |
| `Segmented` · `Tabs` | `options`/`tabs`, `bind:value`; arrow keys move in `Segmented` |
| `Badge` | `tone` ok · warn · err · info · muted · accent; `variant` soft · outline · solid; `dot`, `live` |
| `Chip` | `selected`, `onclick`, `onremove` |
| `Card` | `title`, `icon`, `padding`, `interactive`; snippets `actions`, `footer` |
| `Callout` | `tone`, `title`, `icon`; snippet `actions`. Errors use `role="alert"` |
| `Stat` | `label`, `value`, `detail`, `icon` |
| `List` · `ListRow` | The divided list; `ListRow` takes `active` and `onclick` |
| `Modal` | `bind:open`, `title`, `icon`, `size`; snippet `footer`. Traps focus, closes on Escape and backdrop |
| `Toasts` + `toast` | Mount `Toasts` once; call `toast.ok / err / info(text)` |
| `EmptyState` · `PageHeader` | Icon tile with the brand gradient; page title, subtitle and actions |
| `Icon` · `Spinner` · `Dot` · `Kbd` · `Divider` · `Skeleton` | Small parts |
| `Drawer` | A sheet from the right for looking at one thing beside its list: `bind:open`, `title`, `subtitle`, `width`; snippets `leading`, `actions`, `footer`. Traps focus, closes on Escape |
| `CommandPalette` | ⌘K / Ctrl K. `bind:open`, `commands` (`id`, `label`, `group`, `icon`, `hint`, `keywords`, `run`). Fuzzy search, arrows and Enter, a combobox for screen readers |
| `BrandLogo` | A service's mark (Simple Icons, CC0) on a quiet tile; a plug for anything without one |
| `StatusPill` | `ready`, `starting`, `restarting`, `needs_login`, `failed`, `stopped`, `off`, `review`: a tone, a word and a pulse while something is happening |
| `CodeBlock` | SQL, JSON, JavaScript and Python, highlighted by a tiny tokenizer with no network and no HTML; scrolls, copies |
| `DiffView` | What changed between two texts: word by word for a sentence, line by line for more. Added and removed carry `+`/`−` and a label, not only colour |
| `TreeView` | A tree of runs (`root` from `buildTree`, `selected`, `look`, `now`, `onselect`): rows by depth with face, agent, task, steps, time, a bar for the share of the cost and a state badge. `role="tree"`; arrows move, Left and Right fold, Enter opens |
| `DelegationCard` | One call to `agents.delegate` (`children` rows, `onopen`; snippet `row`): who was asked, state, steps, cost share, live text or answer, and room for an inline approval |
| `Sparkbar` | A small bar for a share of a total (`value`, `max`, `label`); the label is read aloud, the colour is never the only signal |
| `Stepper` | A short flow (`steps`, `current`); done steps carry a check |
| `KeyValue` · `ToggleRow` | Facts as label and value; a setting with a sentence and a switch |
| `tooltip` · `trapFocus` | Actions. `tooltip` is plain text only |

**Tones.** Never write tone classes by hand: `toneClass(tone, kind)` returns the Tailwind classes for `text`, `dot`, `soft`,
`border`, `solid` or `callout`. State-to-tone mapping stays in `format.ts` (`stateTone`).

**Icons.** A Lucide-style path on a 24px grid in `src/lib/ui/icons.ts`; add one line to add an icon. They are static
markup written in that file and never come from user input.

## Do and don't

- **Do** use `Field` around every control so labels and errors are connected.
- **Do** give icon-only buttons a `label`.
- **Do** keep a solid accent to the one thing a screen wants you to do.
- **Don't** invent colours or sizes in a screen; add a token or use a scale step.
- **Don't** use `faint` for text people must read.
- **Don't** rely on colour alone to say a state.

## Accessibility

- Contrast is tested: `fg` ≥ 7, `muted` and every status colour and `accent` ≥ 4.5, `faint` ≥ 3, and the text on a solid
  accent ≥ 4.5, on every surface in both themes (`src/lib/contrast.test.ts` reads `app.css`).
- Focus is always visible: a 2px accent outline, and a soft ring on fields.
- Dialogs trap focus and give it back; Escape closes them. Toasts are announced politely, errors assertively.
- Motion respects `prefers-reduced-motion`.
- Scrollable regions (`CodeBlock`, `DiffView`) take focus so the keyboard can scroll them.
- Glass never carries text that would not pass on the solid fallback.

## Changing the system

The design system is owned by the package in `design/`; the app does not edit it.

1. Change the design system (its artifact), export the package, and replace the contents of `design/` (`tokens.json`, `tokens.css`,
   `components/`, `api/`, `surfaces.md`, `manifest.json`, `README.md`, `assets/`). Re-render `design/screens/*.png` from the screens
   bundle when the screens changed. Fonts are not vendored twice: the app bundles them.
2. Run `node tools/design-tokens.mjs` to regenerate the token block of `app.css`, then `npm test` (the token test and the contrast
   test say if a value drifted or a pair became unreadable).
3. Update the kit components the change touches, open the **Design system** screen in both themes, and look.
4. Retake the two screenshots of that screen: serve with `VITE_START_VIEW=design VITE_START_THEME=light npx vite` (and `dark`) and
   capture the full page with a headless browser.

When the artifact cannot be reached, a screen the phase needs is specified in `design/` in the same form (README, preview,
`manifest.json`, `surfaces.md`, a PNG in `design/screens/`) and logged in `design/local-additions.md`, to be ported back into the
artifact. The screen is built only after it is specified there.

## Known differences

Where the app deliberately differs from `design/screens`, and why. Each line is removed when the reason goes away.

| Screen | Difference | Reason |
|---|---|---|
| Sidebar | Command Center and Marketplace are not listed | Their phases are not shipped; a link would lead nowhere |
| Toolbar, sidebar | No account avatar, no daily budget beside "Spend today" | No accounts or budget setting yet ([roadmap](roadmap.md#across-phases), Command Center) |
| Run | "This run" lists the model, who started it and the cost, not the grants, what asks first and the limits | The run record does not return its grants and limits yet; the roadmap lists them |
| Run | The work tree is the kit's TreeView (state, steps, time, cost bar), not a four-column table | Same facts; TreeView adds folding and keyboard navigation |
| Documents | A *Watch* / *Stop watching* button sits beside *Sync* on each folder, and the Add-folder dialog has a *Keep it up to date* switch | The design shows the *Watching* badge only; a person has to be able to turn it on for a folder that was added before, and off |
| New run | Budget, Time and Steps sit in a *Limits* card of their own (not in the Activity card); no "Always for this run"; no model field beside them, because the model is chosen in its own card | The rest of the form was built before the limits existed; "Always for this run" waits for the Command Center |
| Providers | No price per million tokens, p95 latency or spend; routes show their names only | Prices and timings are not stored per model; routes live in `providers.toml` |
| Integrations | Cards without a 24-hour history strip; filters are All and Needs attention, and Native sits on its own tab | No health history is recorded yet; `HealthGrid` is in the kit for when it is |
| Workflows | The palette also lists *After workflow* and *Manual* under Triggers; it has no *Pull request* action and no *Add a task* action | Both trigger kinds exist in the model and the TriggerChip; no tool opens pull requests yet, and a workflow step that makes a task is left for later |
| Workflows | *Time saved* shows a dash until a workflow says how many minutes a run saves; *Waiting for people* names the oldest wait, not an on-call rotation | Nothing is estimated or invented; people and rotations arrive with Phase 16 |
| Workflow editor | *Who decides* is not a field: an approval is answered by the person using ZORIX. The On switch sits in the settings of the workflow, and a *Runs* button sits beside *Versions* | There are no accounts or rotations yet; the run view opens from the Runs drawer, as the design system says |
| Workflows list | Bars are as tall as the run was long (square root, at least 40%) | The design system draws bars of different heights without saying why; duration is the one real figure |
| Memory | A *To approve* tab replaces the proposals list; no *Scopes* tab | Proposals are approvals of `memory.remember`, answered here or in the run |
| Agents | No evaluation score on a card, no Mine, Templates or Archived filters | The card does not carry the latest score, where an agent came from, or archived agents |
| Agent chat | The rail shows the team or the agent and the cost so far, not *Sources cited* or *Memory used* | Citations and the memory a chat used are not exposed per conversation yet |
| Project board, timeline | No *Share* button; *Ask the manager* (when the project has one) and a settings button stand in its place | There are no accounts or other people until Phase 16 |
| Projects | A person is a round avatar with a person icon, not initials; the *People* filter shows the tasks you have | ZORIX does not know a name to take initials from, and the only person is you |
| Projects | An agent's face is the emoji and colour of its record, not an icon | The kit's Avatar draws agents that way everywhere |
| Board | A card drops its due date for a deliverables count once it is Done, and its parts wrap in a narrow lane | The design system shows the Done count and no completion date; a task does not record when it finished |
| Board, list | A lane has the kit's 16 px radius, not 14 | One radius for cards and lanes in the kit |
| Task | The drawer edits the task in place (fields, dependencies, links, deliverables), and its Stepper only shows where the task is | The design system draws a read-out; moves are the footer buttons, the board and the keyboard |
| Timeline | Tasks with no dates sit in a *Not scheduled* row under the grid; arrows are drawn between a task and what it waits for | Added to the design system for this phase, logged in `design/local-additions.md` |
| Workforce | You are the card at the top, and who answers to you hangs below; the design system draws you among the manager's reports. Chips start at the fourth level, and a card is 120 px high so a two-line role fits | Every chain ends at you ([ADR-0038](adr/0038-workers-are-records-over-agents.md)), so the chart is drawn from the person who approves |
| Workforce | A manager's card shows its on-time rate, a worker that waits shows how many, otherwise the eval score or the on-time rate; *Waiting for people* says "N approvals · M reviews", not "N for you" | The only person is you, so "for you" would say nothing; what is shown is what is measured |
| Workforce, worker | Only *Spend this week* has a sparkline and a change; the other tiles have none. One switch turns on both the Monday plan and the Friday report | ZORIX records spend by day, not tasks or on-time rate; the schedule is one setting for a manager |
| Workforce, kit | Radii and text sizes are the nearest step of the token scale (cards 12 px, figures 11.5 px), and the dial's tiles are 8 px | The design lint accepts tokens and scale steps only |
| Sidebar | The foot says "N workers busy" before "waiting for you", and has no daily budget | A daily budget arrives with the Command Center (Phase 16) |
| Inbox | A second section, *From your workforce*, lists plans and reports, proposals and calls that ran without asking, apart from what waits for a decision | The design system has no Inbox for notices; they are information, not asks, so they do not count in the bell |

### Contrast in the design system itself

Two pairs in the design system are under 4.5:1, and the app follows it until the design system changes: a warn word on its own 12–14% tint in the light theme (a "Needs your approval" badge, about 4.0:1) and an accent word on the accent tint in the dark theme (the current sidebar entry, about 4.2:1). Everything the app adds on top uses a solid warn fill with `bg` text (4.65:1) or `muted` text instead.
