import assert from "node:assert/strict";
import { test } from "node:test";

import { highlight, markJsonKeys } from "./highlight.ts";

const join = (code: string, lang: Parameters<typeof highlight>[1]) => highlight(code, lang).map((t) => t.text).join("");

test("tokens always join back to the exact text, whatever it is", () => {
  const samples = [
    "SELECT id, 'it''s' AS name -- note\nFROM t WHERE n >= 10.5e3;",
    "const a = `x ${1}`; // done\nfunction f(a) { return a?.b ?? 'q'; }",
    'def f(x):\n    """doc"""\n    return x # c\nprint("unterminated',
    '{"a": [1, -2.5, true, null], "b": "c\\"d"}',
    "'unterminated string",
    "",
    "é中\u{1f600} weird \u0000 bytes",
  ];
  for (const lang of ["sql", "js", "py", "json", "text"] as const) {
    for (const sample of samples) assert.equal(join(sample, lang), sample, `${lang}: ${JSON.stringify(sample)}`);
  }
});

test("sql keywords are found in any case, and strings and comments are kept whole", () => {
  const tokens = highlight("select count(*) FROM t -- all", "sql");
  const kinds = Object.fromEntries(tokens.filter((t) => t.kind !== "plain" && t.kind !== "punct").map((t) => [t.text, t.kind]));
  assert.equal(kinds.select, "kw");
  assert.equal(kinds.FROM, "kw");
  assert.equal(kinds["-- all"], "com");
  assert.equal(highlight("SELECT 'a;b'", "sql").find((t) => t.kind === "str")?.text, "'a;b'");
});

test("python and javascript tell keywords, functions, numbers and comments apart", () => {
  const py = highlight("def run(x):  # go\n    return 42", "py");
  assert.ok(py.some((t) => t.kind === "kw" && t.text === "def"));
  assert.ok(py.some((t) => t.kind === "fn" && t.text === "run"));
  assert.ok(py.some((t) => t.kind === "com" && t.text === "# go"));
  assert.ok(py.some((t) => t.kind === "num" && t.text === "42"));
  const js = highlight("const total = sum(1, 2);", "js");
  assert.ok(js.some((t) => t.kind === "kw" && t.text === "const"));
  assert.ok(js.some((t) => t.kind === "fn" && t.text === "sum"));
});

test("json keys are marked apart from string values", () => {
  const tokens = markJsonKeys(highlight('{"name": "zorix", "n": 1}', "json"));
  assert.equal(tokens.find((t) => t.text === '"name"')?.kind, "fn");
  assert.equal(tokens.find((t) => t.text === '"zorix"')?.kind, "str");
});

test("a very long line does not take long", () => {
  const started = Date.now();
  highlight("x".repeat(200_000) + "'" + "y".repeat(200_000), "js");
  assert.ok(Date.now() - started < 2000);
});
