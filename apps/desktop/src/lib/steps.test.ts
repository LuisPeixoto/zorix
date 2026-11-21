import assert from "node:assert/strict";
import { test } from "node:test";

import type { ToolCall } from "./api.ts";
import { providerCount, runSteps } from "./steps.ts";

const call = (tool: string): ToolCall => ({ call_id: tool, tool, arguments: {}, status: "succeeded", attempts: 1, error: null });
const step = (kind: string, name: string, provider: string | null = null) => ({ kind, name, provider, duration_ms: 10 });

test("tool steps take their calls in order and model steps keep the provider", () => {
  const items = runSteps([step("model", "opus", "anthropic"), step("tool", "?"), step("tool", "?"), step("model", "opus", "anthropic")], [call("documents.search"), call("agents.delegate")]);
  assert.deepEqual(items.map((i) => i.title), ["anthropic/opus", "documents.search", "agents.delegate", "anthropic/opus"]);
  assert.equal(items[2].kind === "tool" && items[2].call?.tool, "agents.delegate");
});

test("a tool step whose call is not recorded yet has no call", () => {
  const items = runSteps([step("tool", "clock.now")], []);
  assert.equal(items[0].kind === "tool" && items[0].call, null);
  assert.equal(items[0].title, "clock.now");
});

test("providers are counted once each", () => {
  assert.equal(providerCount([step("model", "a", "anthropic"), step("model", "b", "anthropic"), step("model", "c", "openai"), step("tool", "x")]), 2);
  assert.equal(providerCount([]), 0);
});
