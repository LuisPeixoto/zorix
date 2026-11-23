import assert from "node:assert/strict";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { pseudoCatalogue } from "./core.ts";
import { en } from "./messages.en.ts";

const src = join(dirname(fileURLToPath(import.meta.url)), "../..");
const walk = (dir: string): string[] =>
  readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return name === "node_modules" ? [] : walk(path);
    return /\.(svelte|ts)$/.test(name) && !/\.test\.ts$/.test(name) && !path.endsWith("messages.en.ts") ? [path] : [];
  });
const sources = walk(src).map((path) => readFileSync(path, "utf8")).join("\n");

test("every message is used somewhere, so the catalogue does not collect dead sentences", () => {
  const unused = Object.keys(en).filter((key) => !sources.includes(`"${key}"`));
  assert.deepEqual(unused, []);
});

test("a message with plural forms has `other` and only plural categories", () => {
  for (const [key, message] of Object.entries(en)) {
    if (typeof message === "string") continue;
    assert.ok(typeof message.other === "string" && message.other.length > 0, `${key} has no \`other\``);
    // A form may leave the number out when the sentence only agrees with it ("{name} is" / "{names} are"), but it is chosen by `count`.
    assert.ok(Object.keys(message).every((form) => ["zero", "one", "two", "few", "many", "other"].includes(form)), `${key} has a form that is not a plural category`);
  }
});

test("placeholders are written the same way everywhere and a message is not an empty sentence", () => {
  for (const [key, message] of Object.entries(en)) {
    const texts = typeof message === "string" ? [message] : Object.values(message);
    for (const text of texts) {
      assert.ok(text.trim().length > 0 || key.endsWith("Empty"), `${key} is empty`);
      assert.ok(!/\{\s|\s\}/.test(text.replace(/\{\{.*?\}\}/g, "")), `${key} has a placeholder with spaces: ${text}`);
    }
  }
});

test("the pseudo-locale touches every message, so a string that skips the catalogue is the one that looks plain", () => {
  const pseudo = pseudoCatalogue(en);
  for (const key of Object.keys(en)) {
    const message = pseudo[key];
    const texts = typeof message === "string" ? [message] : Object.values(message);
    for (const text of texts) assert.match(text, /^\[.*\]$/s, key);
  }
});
