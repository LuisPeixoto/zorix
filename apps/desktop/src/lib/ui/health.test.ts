import assert from "node:assert/strict";
import { test } from "node:test";

import { type HealthItem, describeDay, lastDay, sortFailingFirst } from "./health.ts";

const item = (name: string, state: HealthItem["state"]): HealthItem => ({ id: name, name, state, history: [], note: "" });

test("failing cells come first, then what needs a person, then what is fine, each by name", () => {
  const order = sortFailingFirst([item("Postgres", "ready"), item("Slack", "failed"), item("GitHub", "ready"), item("Linear", "needs_login"), item("Notion", "starting")]);
  assert.deepEqual(order.map((i) => i.name), ["Slack", "Linear", "Notion", "GitHub", "Postgres"]);
});

test("sorting does not change its input", () => {
  const input = [item("b", "ready"), item("a", "failed")];
  sortFailingFirst(input);
  assert.deepEqual(input.map((i) => i.name), ["b", "a"]);
});

test("a day is always 24 bars, padded on the old side", () => {
  const day = lastDay(["err", "ok"]);
  assert.equal(day.length, 24);
  assert.equal(day[0], "none");
  assert.deepEqual(day.slice(-2), ["err", "ok"]);
  assert.equal(lastDay(Array(30).fill("ok")).length, 24);
});

test("the strip reads aloud with its counts", () => {
  const history = [...Array(21).fill("ok"), "warn", "err", "err"];
  assert.equal(describeDay(history), "Last 24 hours: 21 fine, 1 degraded, 2 failing.");
  assert.match(describeDay([]), /24 without data/);
});
