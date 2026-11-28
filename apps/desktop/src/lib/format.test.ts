import assert from "node:assert/strict";
import { test } from "node:test";

import type { ExecEvent } from "./api.ts";
import { ago, describeEvent, duration, eventIcon, isFinished, isLive, isNoise, keyLabel, keyTone, servedBy, shorten, stateLabel, stateTone, tokens } from "./format.ts";

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

test("waiting for sub-runs reads as work in progress, not as a stop", () => {
  assert.equal(stateLabel("waiting_for_children"), "Waiting for sub-runs");
  assert.equal(stateTone("waiting_for_children"), "info");
  assert.ok(!isFinished("waiting_for_children") && !isLive("waiting_for_children"), "nobody may pause it: it is suspended");
  assert.deepEqual(describeEvent(event("execution.waiting_children", { children: ["a", "b"] })).detail, "2 tasks at the same time");
  assert.equal(describeEvent(event("execution.child.settled", { agent: "writer", state: "failed", steps: 1 })).tone, "err");
  assert.equal(describeEvent(event("execution.child.started", { agent: "researcher", task: "Find it." })).title, "researcher started");
  assert.equal(eventIcon("execution.child.settled"), "branch");
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
  const withRequest = event("execution.tool.completed", { tool: "t", duration_ms: 3, server_requests: [{ server: "files", kind: "model", outcome: "answered" }] });
  assert.equal(describeEvent(withRequest).detail, "3 ms · files asked for the model: answered");
  assert.equal(describeEvent(event("execution.model.responded", { tool_calls: 0 })).detail, "final answer");
  assert.equal(describeEvent(event("execution.model.responded", { tool_calls: 2 })).detail, "wants 2 tool calls");
  assert.equal(
    describeEvent(event("execution.model.fallback", { from: "a/x", to: "b/y", reason: "rate_limited" })).detail,
    "a/x → b/y (rate_limited)",
  );
  const summarised = describeEvent(event("execution.context.summarised", { folded_messages: 8, summary_chars: 900 }));
  assert.equal(summarised.title, "Summarised the earlier turns");
  assert.match(summarised.detail, /8 messages became a summary of 900 characters/);
  assert.equal(describeEvent(event("execution.future.thing")).title, "future.thing", "unknown kinds still show up");
});

test("bookkeeping events are hidden from the timeline", () => {
  assert.ok(isNoise(event("execution.progress")));
  assert.ok(!isNoise(event("execution.tool.started")));
});

test("the model that answered is shown with its provider, and key states have words", () => {
  const d = describeEvent(event("execution.model.responded", { tool_calls: 0, provider: "ollama", model: "qwen3:8b" }));
  assert.equal(d.detail, "final answer · ollama/qwen3:8b");
  assert.equal(describeEvent(event("execution.model.responded", { tool_calls: 2 })).detail, "wants 2 tool calls");
  assert.equal(servedBy(["a/x", "b/y"]), "a/x, b/y");
  assert.equal(keyLabel("missing"), "No key yet");
  assert.equal(keyLabel("unavailable"), "Keychain unavailable");
});

test("a key state has a tone and each kind of event has an icon", () => {
  assert.equal(keyTone("present"), "ok");
  assert.equal(keyTone("missing"), "warn");
  assert.equal(keyTone("unavailable"), "err");
  assert.equal(keyTone("not_needed"), "muted");
  assert.equal(eventIcon("execution.tool.completed"), "wrench");
  assert.equal(eventIcon("execution.model.fallback"), "route");
  assert.equal(eventIcon("execution.context.summarised"), "layers");
  assert.equal(eventIcon("execution.model.responded"), "cpu");
  assert.equal(eventIcon("execution.approval.granted"), "shield");
  assert.equal(eventIcon("execution.completed"), "check-circle");
  assert.equal(eventIcon("execution.something.new"), "info");
});
