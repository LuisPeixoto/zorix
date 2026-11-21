// WCAG contrast, and a reader for the colour tokens in app.css, so a test can hold the design to a standard.

export type Rgb = [number, number, number];

export function parseHex(hex: string): Rgb {
  const h = hex.trim().replace(/^#/, "");
  const full = h.length === 3 ? [...h].map((c) => c + c).join("") : h;
  if (!/^[0-9a-fA-F]{6}$/.test(full)) throw new Error(`not a hex colour: ${hex}`);
  return [0, 2, 4].map((i) => parseInt(full.slice(i, i + 2), 16)) as Rgb;
}

function channel(value: number): number {
  const c = value / 255;
  return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

export function luminance([r, g, b]: Rgb): number {
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

/** The WCAG contrast ratio between two colours, from 1 to 21. */
export function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(parseHex(a)), luminance(parseHex(b))].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** The solid (hex) tokens of a block of CSS such as `:root { --bg: …; }`. Tints made with color-mix are left out. */
export function readTokens(block: string): Record<string, string> {
  const tokens: Record<string, string> = {};
  for (const m of block.matchAll(/--([a-z0-9-]+):\s*(#[0-9a-fA-F]{3,6})\s*;/g)) tokens[m[1]] = m[2];
  return tokens;
}

/** The token blocks of app.css: the light theme under `:root`, the dark one under `:root[data-theme="dark"]`. */
export function readThemes(css: string): { light: Record<string, string>; dark: Record<string, string> } {
  const dark = css.match(/:root\[data-theme="dark"\]\s*\{([\s\S]*?)\n\}/);
  const light = css.match(/\n:root\s*\{([\s\S]*?)\n\}/);
  if (!light || !dark) throw new Error("app.css has no light and dark token blocks");
  return { light: readTokens(light[1]), dark: readTokens(dark[1]) };
}
