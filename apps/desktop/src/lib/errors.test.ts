import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { errorText } from "./errors.ts";
import { en } from "./i18n/messages.en.ts";
import { useLocale } from "./i18n/index.ts";

const here = dirname(fileURLToPath(import.meta.url));

test("an error with a code is written from the catalogue, with its values", () => {
  assert.equal(errorText({ code: "not_found.execution", args: { id: "01abc" }, message: "x" }), "Execution 01abc was not found.");
  assert.equal(errorText({ code: "invalid", args: { detail: "Say who is deciding." }, message: "x" }), "Invalid request: Say who is deciding.");
  assert.equal(errorText({ code: "message", args: { detail: "The folder cannot be used." }, message: "x" }), "The folder cannot be used.");
});

test("a code from a newer backend shows its English sentence, and other things reject as text as before", () => {
  assert.equal(errorText({ code: "brand.new_code", args: {}, message: "A new kind of failure." }), "A new kind of failure.");
  assert.equal(errorText("a plain string"), "a plain string");
  assert.equal(errorText(new Error("boom")), "boom");
});

test("the pseudo-locale writes an error too", () => {
  useLocale("en-XA");
  assert.match(errorText({ code: "model.cancelled", args: {}, message: "x" }), /^\[.*\]$/);
  useLocale("en");
});

test("the catalogue has a message for every code the backend can send, word for word", () => {
  const list: [string, string][] = JSON.parse(readFileSync(join(here, "../../../../crates/zorix-core/reasons.json"), "utf8"));
  assert.ok(list.length >= 20);
  for (const [code, template] of list) {
    assert.equal((en as Record<string, unknown>)[`reason.${code}`], template, `reason.${code} differs from the backend's sentence`);
  }
  // And no message of the catalogue is a reason the backend no longer has.
  const known = new Set(list.map(([code]) => `reason.${code}`));
  for (const key of Object.keys(en)) if (key.startsWith("reason.")) assert.ok(known.has(key), `${key} is not a code of the backend`);
});
