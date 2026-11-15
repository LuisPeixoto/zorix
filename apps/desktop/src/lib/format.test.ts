import assert from "node:assert/strict";
import { test } from "node:test";

import type { ExecEvent } from "./api.ts";
import { ago, describeEvent, duration, isFinished, isLive, isNoise, shorten, stateLabel, stateTone, tokens } from "./format.ts";

const event = (kind: string, payload: Record<string, unknown> = {}): ExecEvent => ({
  seq: 1,
  execution_id: "x",
  at: 0,
  schema_version: 1,
  kind,
  payload,
});

test("states have a label and a tone, and approval stands out", () => {
  assert.equal(stateLabel("waiting_for_approval"), "Needs your approval");
  assert.equal(stateTone("waiting_for_approval"), "warn");
  assert.equal(stateTone("completed"), "ok");
  assert.equal(stateTone("failed"), "err");
  assert.equal(stateTone("paused"), "muted");
});

test("live and finished states do not overlap", () => {
  for (const state of ["queued", "running", "waiting_for_model", "waiting_for_tool"] as const) {
    assert.ok(isLive(state) && !isFinished(state), state);
  }
  for (const state of ["completed", "failed", "cancelled"] as const) {
    assert.ok(isFinished(state) && !isLive(state), state);
  }
  assert.ok(!isLive("waiting_for_approval") && !isLive("paused"));
});

test("durations read naturally", () => {
  assert.equal(duration(null), "–");
  assert.equal(duration(12), "12 ms");
  assert.equal(duration(1500), "1.5 s");
  assert.equal(duration(42_000), "42 s");
  assert.equal(duration(125_000), "2 min 5 s");
});

test("relative times", () => {
  const now = 1_000_000_000;
  assert.equal(ago(now - 1000, now), "just now");
  assert.equal(ago(now - 30_000, now), "30 s ago");
  assert.equal(ago(now - 5 * 60_000, now), "5 min ago");
  assert.equal(ago(now - 3 * 3_600_000, now), "3 h ago");
  assert.equal(ago(now - 2 * 86_400_000, now), "2 d ago");
  assert.equal(ago(now + 5000, now), "just now");
});

test("text is flattened and shortened", () => {
  assert.equal(shorten("a\n  b\tc", 20), "a b c");
  assert.equal(shorten("abcdefghij", 5), "abcd…");
  assert.equal(tokens({ input_tokens: 40, output_tokens: 2 }), 42);
});

test("events are described for people", () => {
  const view = describeEvent(event("execution.tool.requested", { tool: "math.evaluate", arguments: { expression: "17*23" } }));
  assert.equal(view.title, "Tool requested: math.evaluate");
  assert.equal(view.detail, '{"expression":"17*23"}');
  assert.equal(describeEvent(event("execution.waiting_approval", { tool: "notes.put", reason: "asks every time" })).tone, "warn");
  assert.equal(describeEvent(event("execution.tool.completed", { tool: "t", duration_ms: 3 })).detail, "3 ms");
  assert.equal(describeEvent(event("execution.model.responded", { tool_calls: 0 })).detail, "final answer");
  assert.equal(describeEvent(event("execution.model.responded", { tool_calls: 2 })).detail, "wants 2 tool calls");
  assert.equal(
    describeEvent(event("execution.model.fallback", { from: "a/x", to: "b/y", reason: "rate_limited" })).detail,
    "a/x → b/y (rate_limited)",
  );
  assert.equal(describeEvent(event("execution.future.thing")).title, "future.thing", "unknown kinds still show up");
});

test("bookkeeping events are hidden from the timeline", () => {
  assert.ok(isNoise(event("execution.progress")));
  assert.ok(!isNoise(event("execution.tool.started")));
});
