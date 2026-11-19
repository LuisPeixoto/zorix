import assert from "node:assert/strict";
import { test } from "node:test";

import { fuzzyFilter, fuzzyScore } from "./fuzzy.ts";

test("letters must appear in order", () => {
  assert.equal(fuzzyScore("xyz", "memory"), null);
  assert.notEqual(fuzzyScore("mry", "memory"), null);
  assert.equal(fuzzyScore("ym", "memory"), null);
});

test("an empty query keeps everything in its order", () => {
  assert.deepEqual(fuzzyFilter(["b", "a"], "  ", (x) => x), ["b", "a"]);
});

test("a prefix beats a word start beats a scatter", () => {
  const order = fuzzyFilter(["Open the memory", "Memory", "my extra memo ry"], "mem", (x) => x);
  assert.equal(order[0], "Memory");
  assert.equal(order[1], "Open the memory");
});

test("matching ignores case and keeps ties stable", () => {
  assert.deepEqual(fuzzyFilter(["Alpha", "alpine"], "AL", (x) => x), ["Alpha", "alpine"]);
  assert.notEqual(fuzzyScore("GH", "GitHub"), null);
});
