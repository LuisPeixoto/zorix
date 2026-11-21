# Icon

Lucide-style strokes on a 24px grid with a round 1.8px stroke, drawn as static markup from one file.

- Size 15 in buttons and rows, 14 in small buttons, 13 in badges, 20 in tiles, 28 in empty states.
- Icons take `currentColor`: colour them with a text token (`muted` at rest, `accent` when active, a status token beside a status word).
- Add an icon by adding one path to `icons.ts`; there is no icon library and no icon comes from user input.
- The icons outlined in violet are the v3 set added for workflows, projects, the workforce, the Command Center and the Marketplace.
- An icon-only control needs a text label (see IconButton). An icon never carries a state by itself.
