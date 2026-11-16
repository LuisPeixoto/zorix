import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import { contrast, readThemes } from "./contrast.ts";

const css = readFileSync(new URL("../app.css", import.meta.url), "utf8");
const themes = readThemes(css);

test("both themes define every solid token", () => {
  const required = ["bg", "panel", "panel-2", "elevated", "hover", "line", "line-strong", "fg", "muted", "faint", "accent", "accent-2", "accent-fg", "ok", "warn", "err", "info"];
  for (const [name, tokens] of Object.entries(themes)) {
    for (const key of required) assert.ok(tokens[key], `${name} theme lacks --${key}`);
  }
});

test("text is readable on every surface, in both themes (WCAG AA)", () => {
  const surfaces = ["bg", "panel", "panel-2", "elevated"];
  for (const [name, t] of Object.entries(themes)) {
    for (const surface of surfaces) {
      const need = (text: string, min: number) =>
        assert.ok(contrast(t[text], t[surface]) >= min, `${name}: --${text} on --${surface} is ${contrast(t[text], t[surface]).toFixed(2)}, needs ${min}`);
      need("fg", 7);
      need("muted", 4.5);
      need("faint", 3); // hints and icons, never body text
      for (const tone of ["ok", "warn", "err", "info", "accent"]) need(tone, 4.5);
    }
  }
});

test("a primary button's text is readable on the accent, and hover rows keep text readable", () => {
  for (const [name, t] of Object.entries(themes)) {
    assert.ok(contrast(t["accent-fg"], t.accent) >= 4.5, `${name}: accent-fg on accent is ${contrast(t["accent-fg"], t.accent).toFixed(2)}`);
    assert.ok(contrast(t.fg, t.hover) >= 7, `${name}: fg on hover`);
    assert.ok(contrast(t.muted, t.hover) >= 4.5, `${name}: muted on hover`);
  }
});

test("contrast helper agrees with known values", () => {
  assert.equal(Math.round(contrast("#000000", "#ffffff")), 21);
  assert.equal(contrast("#777777", "#777777"), 1);
});
