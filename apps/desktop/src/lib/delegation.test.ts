import assert from "node:assert/strict";
import { test } from "node:test";

import type { Execution } from "./api.ts";
import { childIds, childRows, rowState } from "./delegation.ts";

const run = (id: string, state: string, extra: Partial<Execution> = {}) =>
  ({
    id,
    state,
    iteration: 2,
    usage: { input_tokens: 1, output_tokens: 1, cost_micros: 40, model_calls: 1, tool_calls: 0 },
    request: { prompt: `prompt ${id}`, instructions: null, model: "m" },
    agent: { agent: "a", slug: `agent-${id}`, version: 1, hash: "h" },
    result: null,
    failure: null,
    cancel_reason: null,
    ...extra,
  }) as unknown as Execution;
const look = (r: Execution) => ({ name: `Name ${r.id}`, emoji: "🤖", color: "blue" });

test("the children of a call come from what was started, or from what settled, without repeats", () => {
  assert.deepEqual(childIds({ result: { delegated: [{ child: "a" }, { child: "b" }] } }), ["a", "b"]);
  assert.deepEqual(childIds({ result: { results: [{ child_id: "a" }, { child_id: "a" }] } }), ["a"]);
  assert.deepEqual(childIds({ result: null }), []);
});

test("a sub-run that asks for a person shows as waiting, whatever its own state says", () => {
  assert.equal(rowState(run("a", "running"), true), "waiting");
  assert.equal(rowState(run("a", "queued"), false), "starting");
  assert.equal(rowState(run("a", "waiting_for_children"), false), "working");
  assert.equal(rowState(run("a", "completed"), true), "completed");
});

test("rows carry the task, the answer or the reason, and only a working row has live text", () => {
  const rows = childRows(
    { arguments: { tasks: [{ task: "one" }, { task: "two" }, { task: "three" }] }, result: { delegated: [{ child: "a" }, { child: "b" }, { child: "c" }, { child: "gone" }] } },
    [run("a", "completed", { result: { text: "done!", structured: null } }), run("b", "failed", { failure: { kind: "x", message: "boom", limit: null } }), run("c", "running")],
    new Set(),
    look,
    { a: "ignored", c: "typing" },
  );
  assert.deepEqual(rows.map((r) => [r.id, r.task, r.state, r.answer ?? null, r.error ?? null, r.live ?? null]), [
    ["a", "one", "completed", "done!", null, null],
    ["b", "two", "failed", null, "boom", null],
    ["c", "three", "working", null, null, "typing"],
  ]);
});
