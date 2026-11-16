import assert from "node:assert/strict";
import { test } from "node:test";

import { DEFAULT_MODE, nextMode, parseStored, resolve, serialize } from "./theme.ts";

test("light is the default, and bad or missing storage falls back to it", () => {
  assert.equal(DEFAULT_MODE, "light");
  for (const text of [null, "", "nope", "{}", '{"mode":"purple"}', '{"mode":3}', "null"]) {
    assert.equal(parseStored(text), "light", String(text));
  }
});

test("a saved choice survives a round trip", () => {
  for (const mode of ["light", "dark", "system"] as const) assert.equal(parseStored(serialize(mode)), mode);
});

test("system follows the operating system, the others do not", () => {
  assert.equal(resolve("system", true), "dark");
  assert.equal(resolve("system", false), "light");
  assert.equal(resolve("light", true), "light");
  assert.equal(resolve("dark", false), "dark");
});

test("the toggle cycles light, dark, system", () => {
  assert.equal(nextMode("light"), "dark");
  assert.equal(nextMode("dark"), "system");
  assert.equal(nextMode("system"), "light");
});
