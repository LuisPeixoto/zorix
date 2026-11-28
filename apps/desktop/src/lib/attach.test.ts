import assert from "node:assert/strict";
import test from "node:test";

import { type CheckedFile, MAX_ATTACHED, addChecked, estimatedChars, insertMention, mentionAt, withOutsideFlags } from "./attach.ts";

const ok = (name: string, bytes = 10, outside = false): CheckedFile => ({ path: `/w/${name}`, name, bytes, problem: null, outside_workspace: outside });

test("an @ opens a mention at the start, after a space or a line break, and not inside a word or an address", () => {
  assert.deepEqual(mentionAt("@", 1), { start: 0, end: 1, query: "" });
  assert.deepEqual(mentionAt("Compare @con", 12), { start: 8, end: 12, query: "con" });
  assert.deepEqual(mentionAt("line one\n@pl", 12), { start: 9, end: 12, query: "pl" });
  assert.deepEqual(mentionAt("see (@notes", 11), { start: 5, end: 11, query: "notes" });
  assert.equal(mentionAt("write to luis@example.com", 25), null, "an address is not a mention");
  assert.equal(mentionAt("Compare @contract and", 21), null, "the caret is past the word");
  assert.equal(mentionAt("no at sign", 10), null);
  assert.equal(mentionAt("@@", 2), null, "a second @ is not part of a name");
});

test("the caret decides which mention: the text after it is not part of the query", () => {
  assert.deepEqual(mentionAt("Compare @co and more", 11), { start: 8, end: 11, query: "co" });
});

test("choosing a file puts its name where the @ was and the caret after it", () => {
  const mention = mentionAt("Compare @con with last year", 12)!;
  const next = insertMention("Compare @con with last year", mention, "contract-2026.pdf");
  assert.equal(next.text, "Compare @contract-2026.pdf with last year");
  assert.equal(next.text.slice(0, next.caret), "Compare @contract-2026.pdf ");
  const end = mentionAt("@pl", 3)!;
  assert.deepEqual(insertMention("@pl", end, "plan.md"), { text: "@plan.md ", caret: 9 });
});

test("a path is added once, a file that was refused says why, and the limit holds", () => {
  const first = addChecked([], [ok("a.md"), ok("a.md"), ok("b.md", 5, true)]);
  assert.deepEqual(first.files.map((f) => [f.name, f.outside]), [["a.md", false], ["b.md", true]]);
  const refused = addChecked(first.files, [{ path: "/w/.env", name: ".env", bytes: 0, problem: "its name is one that holds secrets", outside_workspace: false }]);
  assert.deepEqual(refused.refused, [{ name: ".env", problem: "its name is one that holds secrets" }]);
  assert.equal(refused.files.length, 2);
  const many = addChecked([], Array.from({ length: MAX_ATTACHED + 3 }, (_, i) => ok(`f${i}.md`)));
  assert.equal(many.files.length, MAX_ATTACHED);
  assert.equal(many.overLimit, true);
  assert.equal(addChecked(many.files, [ok("f0.md")]).overLimit, false, "a file that is already there is not over the limit");
});

test("the text a PDF holds is guessed from its size more carefully than a text file's", () => {
  const file = (name: string, bytes: number) => ({ path: `/w/${name}`, name, bytes, outside: false });
  assert.equal(estimatedChars([file("a.md", 4000), file("b.PDF", 8000)]), 5000);
  assert.equal(estimatedChars([]), 0);
});

test("a new check of the files changes only the flag, and hands back the same list when nothing changed", () => {
  const files = addChecked([], [ok("a.md", 5, false), ok("b.md", 5, true)]).files;
  assert.equal(withOutsideFlags(files, [ok("a.md", 5, false), ok("b.md", 5, true)]), files, "nothing changed: the very same array");
  assert.equal(withOutsideFlags(files, []), files, "a file the check did not mention keeps its flag");
  const moved = withOutsideFlags(files, [ok("a.md", 5, true), ok("b.md", 5, false)]);
  assert.notEqual(moved, files);
  assert.deepEqual(moved.map((f) => [f.name, f.outside]), [["a.md", true], ["b.md", false]]);
  assert.deepEqual(files.map((f) => f.outside), [false, true], "the old list was not changed");
});
