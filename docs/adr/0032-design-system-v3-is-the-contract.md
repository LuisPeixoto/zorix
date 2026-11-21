# ADR-0032: Design system v3 is the contract for every screen

Status: accepted (design alignment, 0.6.1)

## Context

Until version 0.6 the design system lived in the app: `app.css` and `src/lib/ui/` were both the definition and the implementation.
That worked while one person drew every screen, but it leaves nothing to check a new screen against, and each phase can drift a
little. The design system was then specified as its own package (tokens, 70 component specifications, and 17 assembled screens at
1440 px that show every surface of ZORIX 1.0, including Phases 7 to 11). The app has to follow it, and so must every later phase.

## Decision

- The package is **vendored at `design/`** and is the contract: `tokens.json` and `tokens.css`, `components/<Name>/README.md` with a
  preview, `index.d.ts`, `surfaces.md` (which components each screen is built from), and `design/screens/*.png`, the 17 assembled
  screens. The app implements it; it does not redefine it. `docs/design-system.md` explains the mapping.
- **Tokens are generated.** The token block of `apps/desktop/src/app.css` is written by `tools/design-tokens.mjs` from
  `design/tokens.json`, and `--check` fails when they differ. Tailwind class names stay (`bg-panel`, `text-muted`, `rounded-2xl`); the
  design system's names map onto them.
- **A lint keeps screens on the scale.** `tools/check-design.mjs` rejects colour literals, arbitrary values for colour, type size,
  radius, shadow and duration, durations over 200 ms, and emoji in interface copy. An exception is allowed only in an allowlist, with
  its reason beside it.
- **Kit components follow their README**: name, props, sizes, states and copy rules. A component the app needs and the kit lacks is
  built from its specification, not invented.
- **A screen or component that is not in the design system is added to it first.** A phase that needs one updates the design system
  (the claude.ai artifact), re-vendors `design/`, and only then builds it.
- **An element that needs data ZORIX does not have yet is left out**, and the roadmap lists it under the phase that brings it. Nothing is
  mocked to make a screen look finished (a daily budget with no budget, a latency with no measurement).
- **Navigation shows only shipped surfaces.** The sidebar keeps the design system's groups and order; an item appears when its phase
  ships, so there are no dead links.
- **Definition of done for any change to the interface:**
  - Light and Dark screenshots of the real app at 1440 px, looked at beside the matching `design/screens` image;
  - a design critique and an accessibility review, with their findings fixed;
  - every difference that remains is written down, with its reason, in `docs/design-system.md`.

## Consequences

- The design system can change without touching the app, and the app cannot silently change the design system.
- The gates grow by two quick scripts, and an exception costs a line and a reason, which is the point.
- A phase takes a little longer at the start (design system first), and much less to review at the end.
- Contrast stays covered by `contrast.test.ts`, which reads the generated tokens.
