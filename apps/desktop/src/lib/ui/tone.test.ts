import assert from "node:assert/strict";
import { test } from "node:test";

import { cn } from "./cn.ts";
import { ICONS } from "./icons.ts";
import { TONES, TONE_KINDS, toneClass } from "./tone.ts";

test("every tone has a class for every kind", () => {
  for (const tone of TONES) {
    for (const kind of TONE_KINDS) assert.ok(toneClass(tone, kind).length > 0, `${tone}/${kind}`);
  }
  assert.equal(toneClass("ok"), "text-ok");
  assert.equal(toneClass("err", "dot"), "bg-err");
  assert.ok(toneClass("warn", "soft").includes("text-warn") && !toneClass("warn", "soft").includes("err"));
});

test("cn skips what is empty, flattens arrays and reads objects", () => {
  assert.equal(cn("a", false, null, undefined, "", "b"), "a b");
  assert.equal(cn(["a", [false, "b"]], { c: true, d: false }, 0 > 1 && "x"), "a b c");
});

test("every icon is a non-empty piece of svg markup", () => {
  for (const [name, markup] of Object.entries(ICONS)) {
    assert.match(markup, /^<(path|circle|rect|polygon|line|polyline)/, name);
    assert.ok(!/<script|\son[a-z]+=/i.test(markup), `${name} must be static shapes only`);
  }
});
