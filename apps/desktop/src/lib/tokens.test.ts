import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

// The tokens of app.css are generated from design/tokens.json (tools/design-tokens.mjs). This holds them to it from inside the app's own tests.
const tokens = JSON.parse(readFileSync(new URL("../../../../design/tokens.json", import.meta.url), "utf8"));
// Git may check files out with CRLF line endings (Windows); the patterns below are written for LF.
const css = readFileSync(new URL("../app.css", import.meta.url), "utf8").replace(/\r\n/g, "\n");

const block = (name: string) => {
  const m = css.match(new RegExp(`/\\* design-tokens:${name}:begin \\*/\\n([\\s\\S]*?)\\n\\s*/\\* design-tokens:${name}:end \\*/`));
  assert.ok(m, `app.css lacks the ${name} token block`);
  return m[1];
};
const declared = (text: string) => Object.fromEntries([...text.matchAll(/--([a-z0-9-]+):\s*([^;]+);/g)].map((m) => [m[1], m[2].trim()]));
const resolve = (value: string) => value.replace(/^\{([a-z0-9-]+)\}$/, "var(--$1)");

for (const theme of ["light", "dark"] as const) {
  test(`${theme}: every colour and shadow token equals the design system`, () => {
    const found = declared(block(theme));
    for (const t of [...tokens.color.tokens, ...tokens.shadow.tokens]) {
      assert.equal(found[t.name], resolve(t.value[theme]), `--${t.name} in the ${theme} theme`);
    }
  });
}

test("scales (space, radius, duration, easing) equal the design system", () => {
  const found = declared(block("light"));
  for (const t of [...tokens.spacing.tokens, ...tokens.radius.tokens, ...tokens.duration.tokens, ...tokens.easing.tokens]) {
    assert.equal(found[t.name], t.value, `--${t.name}`);
  }
});

test("nothing is slower than 200 ms", () => {
  for (const t of tokens.duration.tokens) assert.ok(parseInt(t.value, 10) <= 200, `${t.name} is ${t.value}`);
});

test("the Tailwind type scale is the design system's type scale", () => {
  const theme = declared(css.slice(css.indexOf("@theme inline"), css.indexOf("/* Light is the default theme. */")));
  let checked = 0;
  for (const group of tokens.type.groups) {
    for (const style of group.styles) {
      const tw = style.usage.match(/^text-([0-9a-z]+)[.,]/)?.[1];
      if (!tw) continue;
      assert.equal(theme[`text-${tw}`], style.fontSize, `${style.name} is text-${tw}`);
      checked += 1;
    }
  }
  assert.equal(checked, 10, "the ten text styles map onto Tailwind's names");
});

test("Tailwind maps every colour token to a utility", () => {
  const mapping = declared(block("theme"));
  for (const t of tokens.color.tokens.filter((c: { name: string }) => c.name !== "inset")) {
    assert.equal(mapping[`color-${t.name}`], `var(--${t.name})`, `color-${t.name}`);
  }
});

test("the native window background follows the bg token", async () => {
  const { WINDOW_BACKGROUND } = await import("./theme.ts");
  const bg = tokens.color.tokens.find((t: { name: string }) => t.name === "bg");
  assert.equal(WINDOW_BACKGROUND.light, bg.value.light);
  assert.equal(WINDOW_BACKGROUND.dark, bg.value.dark);
});
