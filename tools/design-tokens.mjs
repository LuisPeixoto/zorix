// Writes the design tokens of the desktop app from the design system.
//
//   node tools/design-tokens.mjs           rewrite the generated blocks of apps/desktop/src/app.css
//   node tools/design-tokens.mjs --check   fail if app.css differs from design/tokens.json (used by the gates and CI)
//
// design/tokens.json is the source of truth (ADR-0032). Three blocks of app.css sit between
// `/* design-tokens:<name>:begin */` and `/* design-tokens:<name>:end */` markers and are owned by this script:
//   theme  Tailwind's mapping (`bg-panel`, `text-series-1`, `rounded-2xl`, `shadow-card`, …), inside `@theme inline`
//   light  the light values and the unthemed scales (spacing, radius, duration, easing, fonts), inside `:root`
//   dark   the dark values, inside `:root[data-theme="dark"]`
// Everything else in app.css is the app's own and is left alone.
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const tokensPath = resolve(root, "design/tokens.json");
const cssPath = resolve(root, "apps/desktop/src/app.css");

const tokens = JSON.parse(readFileSync(tokensPath, "utf8"));
const css = readFileSync(cssPath, "utf8");

/** `{accent}` becomes `var(--accent)`; any other value is used as written. */
const resolveValue = (value) => value.replace(/^\{([a-z0-9-]+)\}$/, "var(--$1)");

const themed = (theme) => [...tokens.color.tokens, ...tokens.shadow.tokens].map((t) => `  --${t.name}: ${resolveValue(t.value[theme])};`);

const unthemed = [
  ...tokens.spacing.tokens,
  ...tokens.radius.tokens,
  ...tokens.duration.tokens,
  ...tokens.easing.tokens,
].map((t) => `  --${t.name}: ${t.value};`);

const fonts = [`  --ui-font-sans: ${tokens.type.families.sans};`, `  --ui-font-mono: ${tokens.type.families.mono};`];

const mapping = [
  ...tokens.color.tokens.filter((t) => t.name !== "inset").map((t) => `  --color-${t.name}: var(--${t.name});`),
  ...tokens.shadow.tokens.filter((t) => t.name.startsWith("sh-")).map((t) => `  --shadow-${t.name.slice(3)}: var(--${t.name});`),
  // Literal values: in `@theme inline` a self-reference (`--radius-sm: var(--radius-sm)`) would be circular.
  ...tokens.radius.tokens.map((t) => `  --${t.name}: ${t.value};`),
  ...tokens.easing.tokens.map((t) => `  --${t.name}: ${t.value};`),
];

const blocks = {
  theme: mapping,
  light: [...fonts, ...themed("light"), ...unthemed],
  dark: themed("dark"),
};

let next = css;
for (const [name, lines] of Object.entries(blocks)) {
  const begin = `/* design-tokens:${name}:begin */`;
  const end = `/* design-tokens:${name}:end */`;
  const pattern = new RegExp(`(  ${begin.replace(/[*/]/g, "\\$&")}\\n)[\\s\\S]*?(\\n  ${end.replace(/[*/]/g, "\\$&")})`);
  if (!pattern.test(next)) {
    console.error(`app.css has no ${begin} … ${end} block`);
    process.exit(2);
  }
  next = next.replace(pattern, (_, open, close) => `${open}${lines.join("\n")}${close}`);
}

if (process.argv.includes("--check")) {
  if (next !== css) {
    console.error("apps/desktop/src/app.css is out of date with design/tokens.json. Run: node tools/design-tokens.mjs");
    process.exit(1);
  }
  console.log("design tokens ok (app.css matches design/tokens.json)");
} else if (next !== css) {
  writeFileSync(cssPath, next);
  console.log("app.css updated from design/tokens.json");
} else {
  console.log("app.css already matches design/tokens.json");
}
