import assert from "node:assert/strict";
import { test } from "node:test";

import type { ExecutionState } from "./api.ts";
import { inFilter, runningCount, spendToday, startOfDay, usd } from "./shell.ts";

const usage = (cost_micros: number) => ({ input_tokens: 0, output_tokens: 0, cost_micros, model_calls: 0, tool_calls: 0 });

test("money reads like the design system: two decimals, four under a cent", () => {
  assert.equal(usd(9_800_000), "$9.80");
  assert.equal(usd(0), "$0.00");
  assert.equal(usd(4_200), "$0.0042");
  assert.equal(usd(10_000), "$0.01");
});

test("only runs started today count toward today's spend", () => {
  const now = new Date(2026, 10, 21, 15, 30).getTime();
  const runs = [
    { created_at: now - 1000, usage: usage(300_000) },
    { created_at: startOfDay(now), usage: usage(200_000) },
    { created_at: startOfDay(now) - 1, usage: usage(5_000_000) },
  ];
  assert.equal(spendToday(runs, now), 500_000);
});

test("running means working: waiting for a person or paused is not running", () => {
  const states: ExecutionState[] = ["running", "waiting_for_children", "queued", "waiting_for_approval", "paused", "completed", "failed"];
  assert.equal(runningCount(states.map((state) => ({ state }))), 3);
});

test("filters split the runs into running, waiting for a person, and finished", () => {
  const of = (state: ExecutionState) => ({ state });
  assert.ok(inFilter(of("running"), "running"));
  assert.ok(!inFilter(of("waiting_for_approval"), "running"));
  assert.ok(inFilter(of("waiting_for_approval"), "waiting"));
  assert.ok(inFilter(of("paused"), "waiting"));
  assert.ok(inFilter(of("cancelled"), "finished"));
  assert.ok(inFilter(of("completed"), "all"));
});
