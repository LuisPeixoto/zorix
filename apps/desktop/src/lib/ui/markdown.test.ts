import assert from "node:assert/strict";
import { test } from "node:test";

import { type Block, type Inline, MAX_CHARS, parseInline, parseMarkdown, safeHref, textOf } from "./markdown.ts";

const inline = (text: string): Inline[] => parseInline(text);
const first = (text: string): Block => parseMarkdown(text)[0];

test("only http, https and mailto addresses are links", () => {
  assert.equal(safeHref("https://example.com/a?b=1"), "https://example.com/a?b=1");
  assert.equal(safeHref("mailto:a@b.co"), "mailto:a@b.co");
  for (const bad of ["javascript:alert(1)", "JaVaScRiPt:alert(1)", "data:text/html,<b>", "file:///etc/passwd", "//evil.com", "/relative", "https://exa mple.com", "  ", "java\nscript:x", "vbscript:x"]) {
    assert.equal(safeHref(bad), null, bad);
  }
});

test("emphasis, code and links are read; an unsafe link keeps its words and loses its address", () => {
  assert.deepEqual(inline("a **b** and *c* and `d`"), [
    { t: "text", text: "a " },
    { t: "strong", children: [{ t: "text", text: "b" }] },
    { t: "text", text: " and " },
    { t: "em", children: [{ t: "text", text: "c" }] },
    { t: "text", text: " and " },
    { t: "code", text: "d" },
  ]);
  assert.deepEqual(inline("[docs](https://example.com)"), [{ t: "link", href: "https://example.com", children: [{ t: "text", text: "docs" }] }]);
  assert.deepEqual(inline("[click](javascript:alert(1))"), [{ t: "text", text: "click" }]);
  assert.deepEqual(inline("![logo](https://x.test/a.png)"), [{ t: "text", text: "logo" }], "images are never loaded");
  assert.deepEqual(inline("see https://example.com/page."), [
    { t: "text", text: "see " },
    { t: "link", href: "https://example.com/page", children: [{ t: "text", text: "https://example.com/page" }] },
    { t: "text", text: "." },
  ]);
});

test("html is only text, and snake_case, lone marks and escapes are left alone", () => {
  assert.deepEqual(inline("<script>alert(1)</script>"), [{ t: "text", text: "<script>alert(1)</script>" }]);
  assert.deepEqual(inline("<b onclick=x>hi</b>"), [{ t: "text", text: "<b onclick=x>hi</b>" }]);
  assert.equal(textOf(inline("my_var_name and 2 * 3 * 4")), "my_var_name and 2 * 3 * 4");
  assert.equal(textOf(inline("a \\*not emphasis\\*")), "a *not emphasis*");
  assert.equal(textOf(inline("1 ` 2")), "1 ` 2");
  assert.equal(textOf(inline("**unclosed and *also")), "**unclosed and *also");
});

test("headings, paragraphs, quotes and rules", () => {
  const blocks = parseMarkdown("# Title\n\nFirst line\nsecond line\n\n> quoted\n> text\n\n---\n\n###### small");
  assert.deepEqual(blocks.map((b) => b.t), ["heading", "p", "quote", "hr", "heading"]);
  assert.equal((blocks[0] as { level: number }).level, 1);
  assert.equal((blocks[4] as { level: number }).level, 6);
  assert.deepEqual((blocks[1] as { children: Inline[] }).children.map((n) => n.t), ["text", "br", "text"], "a single newline is a line break");
  assert.equal(first("####### seven").t, "p", "seven marks is not a heading");
});

test("fenced code keeps everything as it is, and an unclosed fence runs to the end", () => {
  const code = first("```rust\nlet x = *y*; // **not bold**\n```") as { lang: string; text: string };
  assert.deepEqual([code.t, code.lang, code.text], ["code", "rust", "let x = *y*; // **not bold**"]);
  const open = parseMarkdown("```\nnever closed\n\nstill code");
  assert.equal(open.length, 1);
  assert.equal((open[0] as { text: string }).text, "never closed\n\nstill code");
  assert.equal((first("~~~\n```\n~~~") as { text: string }).text, "```");
});

test("lists: bullets, numbers from their start, nesting and paragraphs inside items", () => {
  const list = first("- one\n- two\n  - nested\n- three") as Extract<Block, { t: "list" }>;
  assert.equal(list.ordered, false);
  assert.equal(list.items.length, 3);
  const second = list.items[1];
  assert.deepEqual(second.map((b) => b.t), ["p", "list"]);
  const numbered = first("3. c\n4. d") as Extract<Block, { t: "list" }>;
  assert.deepEqual([numbered.ordered, numbered.start, numbered.items.length], [true, 3, 2]);
  const loose = first("- a\n\n  more of a\n- b") as Extract<Block, { t: "list" }>;
  assert.deepEqual(loose.items[0].map((b) => b.t), ["p", "p"]);
});

test("tables need a header and a separator row, and read alignment", () => {
  const table = first("| Name | Qty |\n| :--- | ---: |\n| a | 1 |\n| b | 2 |") as Extract<Block, { t: "table" }>;
  assert.equal(table.t, "table");
  assert.deepEqual(table.align, ["left", "right"]);
  assert.equal(table.rows.length, 2);
  assert.equal(textOf(table.head[0]), "Name");
  assert.equal(first("a | b\nnot a separator").t, "p");
});

test("nesting and size are bounded", () => {
  const deep = parseMarkdown("> ".repeat(40) + "deep");
  let depth = 0;
  let node: Block | undefined = deep[0];
  while (node && node.t === "quote") {
    depth += 1;
    node = node.children[0];
  }
  assert.ok(depth <= 7, `depth ${depth}`);
  const huge = parseMarkdown("word ".repeat(MAX_CHARS));
  const total = huge.map((b) => (b.t === "p" ? textOf(b.children).length : 0)).reduce((a, b) => a + b, 0);
  assert.ok(total <= MAX_CHARS);
  const started = Date.now();
  parseMarkdown("*a ".repeat(30_000));
  parseMarkdown("`a ".repeat(30_000));
  parseMarkdown("[a](".repeat(20_000));
  assert.ok(Date.now() - started < 4000, "pathological input stays fast");
});

test("an answer in the usual shape reads as expected", () => {
  const blocks = parseMarkdown("Water boils at **100 °C** [1].\n\n## Sources\n1. [Boiling point](https://en.wikipedia.org/wiki/Boiling_point)\n\nConfidence: high");
  assert.deepEqual(blocks.map((b) => b.t), ["p", "heading", "list", "p"]);
});
