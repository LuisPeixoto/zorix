import assert from "node:assert/strict";
import { test } from "node:test";
import { fill, pseudo, pseudoCatalogue, render } from "./core.ts";
import { formatsFor } from "./format.ts";
import { fmt, t, useLocale } from "./index.ts";

test("a message is filled in and a missing value shows instead of vanishing", () => {
  assert.equal(fill("Hello {name}, you have {n} new", { name: "Ana", n: 3 }), "Hello Ana, you have 3 new");
  assert.equal(fill("Hello {name}", {}), "Hello {name}");
  assert.equal(fill("Hello {name}", { name: null }), "Hello {name}");
  assert.equal(fill("No placeholders", { a: 1 }), "No placeholders");
});

test("plural forms follow the locale's own rules", () => {
  const tasks = { one: "{count} task", other: "{count} tasks" };
  assert.equal(render(tasks, "en", { count: 1 }), "1 task");
  assert.equal(render(tasks, "en", { count: 0 }), "0 tasks");
  assert.equal(render(tasks, "en", { count: 2 }), "2 tasks");
  // Polish has four forms; a message that only gives `one` and `other` still works there.
  assert.equal(render({ one: "{count} plik", few: "{count} pliki", many: "{count} plików", other: "{count} pliku" }, "pl", { count: 3 }), "3 pliki");
  assert.equal(render({ one: "{count} plik", other: "{count} plików" }, "pl", { count: 3 }), "3 plików");
  // No count, no plural.
  assert.equal(render(tasks, "en", {}), "{count} tasks");
});

test("the pseudo-locale is longer, accented and keeps its placeholders whole", () => {
  const out = pseudo("Open {name} now");
  assert.match(out, /^\[Øpéñ \{name\} ñöw·+\]$/);
  assert.ok(out.length > "Open {name} now".length * 1.3);
  const cat = pseudoCatalogue({ a: "Save", b: { one: "{count} run", other: "{count} runs" } });
  assert.match(cat.a as string, /^\[/);
  assert.match((cat.b as { other: string }).other, /\{count\}/);
});

test("a key that does not exist shows as itself, and the pseudo-locale can be switched on and off", () => {
  // @ts-expect-error: this key is not in the catalogue on purpose
  assert.equal(t("no.such.key"), "no.such.key");
  assert.equal(t("common.cancel"), "Cancel");
  useLocale("en-XA");
  assert.match(t("common.cancel"), /^\[Cáñçél·+\]$/);
  useLocale("en");
  assert.equal(t("common.cancel"), "Cancel");
});

test("money, numbers, lists, dates and times are formatted for the locale", () => {
  const f = formatsFor("en");
  assert.equal(f.usd(0), "$0.00");
  assert.equal(f.usd(1_234_000), "$1.23");
  assert.equal(f.usd(4_200), "$0.0042", "under a cent it shows four decimals");
  assert.equal(f.dollars(25), "$25");
  assert.equal(f.dollars(25.5), "$25.50");
  assert.equal(f.number(1234567), "1,234,567");
  assert.equal(f.percent(0.94), "94%");
  assert.equal(f.list(["a", "b", "c"]), "a, b and c");
  assert.equal(f.list(["a", "b"]), "a and b");
  assert.equal(f.list(["a"]), "a");
  assert.equal(f.list(["a", "b"], "disjunction"), "a or b");
  assert.equal(f.bytes(900), "900 B");
  assert.equal(f.bytes(2048), "2 KB");
  const noon = new Date(2026, 11, 12, 14, 5, 9);
  assert.equal(f.day(noon), "Dec 12");
  assert.equal(f.clock(noon), "14:05");
  assert.equal(f.clock(noon, true), "14:05:09");
  assert.equal(f.weekday(noon), "Sat");
  // Another language writes the same value its own way.
  assert.equal(formatsFor("de").number(1234567.5), "1.234.567,5");
  assert.equal(formatsFor("pt-BR").usd(1_500_000).replace(/\s/g, " "), "US$ 1,50");
});

test("the shared formatters follow the active locale", () => {
  assert.equal(fmt.usd(2_000_000), "$2.00");
});
