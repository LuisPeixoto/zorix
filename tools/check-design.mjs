// Keeps the desktop app on the design system's scale (ADR-0032). Run: `node tools/check-design.mjs`.
//
// Fails on, in apps/desktop/src (.svelte and .ts, tests and the generated token block of app.css excluded):
//   - a colour literal: #hex, rgb(), rgba(), hsl(), hsla(), oklch(), oklab()
//   - an arbitrary Tailwind value for colour, type size, radius, shadow or duration that is not made only of design tokens
//     (var(--token) is fine; text-[13px], bg-[#fff], rounded-[10px], shadow-[0_1px_2px_black] are not)
//   - a transition longer than 200 ms
//   - an emoji in the interface
// A real exception goes in ALLOW below with the reason beside it; an exception without a reason is itself an error.
import { readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const src = join(root, "apps/desktop/src");

/**
 * Allowed exceptions: `file` is relative to apps/desktop/src, `text` must occur in the offending line.
 * Keep this list short. A line leaves it when the design system gains the token.
 */
const ALLOW = [
  { file: "lib/theme.ts", text: "#f5f6fa", reason: "the native window background must be a literal; tokens.test.ts checks it equals the bg token" },
];

const walk = (dir) =>
  readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return name === "node_modules" ? [] : walk(path);
    return /\.(svelte|ts)$/.test(name) && !/\.test\.ts$/.test(name) ? [path] : [];
  });

// Tailwind utilities whose arbitrary value would be a colour, a size of type, a radius, a shadow or a duration.
const UTILITY = /(?<![\w-])((?:[a-z0-9-]+:)*)(text|bg|border(?:-[trblxy])?|ring|ring-offset|shadow|rounded(?:-[a-z]{1,2})?|duration|delay|ease|fill|stroke|outline|from|via|to|decoration|accent|caret|divide|placeholder|inset-shadow|drop-shadow)-\[([^\]]+)\]/g;
const COLOUR_FN = /\b(?:rgba?|hsla?|oklch|oklab)\(/;
const HEX = /(?<![\w&/])#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6}|[0-9a-fA-F]{3,4})\b/;
// Pictographs, but not the keyboard glyphs ⌘ ⌥ ⇧ ⌃ that name shortcuts.
const EMOJI = /(?![⌘⌥⇧⌃])\p{Extended_Pictographic}/u;

/** True when an arbitrary value is made only of tokens, like `image:var(--accent-grad)` or `var(--accent)`. */
const onlyTokens = (value) => {
  const stripped = value.replace(/^(?:image|color|length|url):/, "");
  return /^(?:var\(--[a-z0-9-]+\)|_|[0-9.]+(?:px|rem|%)?_?)+$/.test(stripped) && /var\(--/.test(stripped) && !COLOUR_FN.test(stripped);
};

const problems = [];
const used = new Set();
const allowed = (file, line) => {
  const i = ALLOW.findIndex((a) => a.file === file && line.includes(a.text));
  if (i >= 0) used.add(i);
  return i >= 0;
};

for (const path of walk(src)) {
  // Forward slashes on every system, so the allowlist reads the same on Windows.
  const file = relative(src, path).replaceAll("\\", "/");
  readFileSync(path, "utf8")
    .split("\n")
    .forEach((line, index) => {
      if (allowed(file, line)) return;
      const at = (message) => problems.push(`${file}:${index + 1}: ${message}`);
      const code = line.replace(/\/\/.*$/, (m) => (/["'`]\s*$/.test(line.slice(0, line.length - m.length)) ? m : ""));
      if (HEX.test(code)) at(`colour literal ${code.match(HEX)[0]}: use a colour token`);
      if (COLOUR_FN.test(code)) at(`colour function ${code.match(COLOUR_FN)[0]}…): use a colour token`);
      for (const m of code.matchAll(UTILITY)) {
        // A border, ring or outline width (`border-[1.5px]`) is a size, not a colour: it is allowed.
        if (/^(?:border|ring|outline)/.test(m[2]) && /^[0-9.]+(?:px)?$/.test(m[3])) continue;
        if (!onlyTokens(m[3])) at(`arbitrary value ${m[2]}-[${m[3]}]: use a token or a scale step`);
      }
      for (const m of code.matchAll(/(?<![\w-])duration-(\d+)(?![\w\]])/g)) {
        if (Number(m[1]) > 200) at(`duration-${m[1]} is longer than 200 ms`);
      }
      for (const m of code.matchAll(/\bduration:\s*(\d+)/g)) {
        if (Number(m[1]) > 200) at(`duration ${m[1]} is longer than 200 ms`);
      }
      // Agent avatars carry an emoji their author chose (design system README), so a line about an emoji or an avatar is data, not copy.
      if (EMOJI.test(code) && !/emoji|avatar/i.test(code)) at(`emoji ${code.match(EMOJI)[0]} in the interface`);
    });
}

ALLOW.forEach((a, i) => {
  if (!a.reason || a.reason.trim().length < 10) problems.push(`allowlist entry ${a.file} “${a.text}” has no reason`);
  if (!used.has(i)) problems.push(`allowlist entry ${a.file} “${a.text}” matches nothing: remove it`);
});

if (problems.length > 0) {
  console.error(problems.join("\n"));
  console.error(`\n${problems.length} design problem(s). See docs/design-system.md#the-contract.`);
  process.exit(1);
}
console.log(`design lint ok (${walk(src).length} files)`);
