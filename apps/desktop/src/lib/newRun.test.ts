import assert from "node:assert/strict";
import { test } from "node:test";

import { destination, grantSummary, providerName } from "./newRun.ts";

const models = [
  { reference: "anthropic/claude-opus-5-5", local: false },
  { reference: "ollama/qwen3:32b", local: true },
];

test("no model at all means the built-in demo", () => {
  assert.deepEqual(destination("", null, models), { kind: "demo" });
  assert.deepEqual(destination("demo", "anthropic/claude-opus-5-5", models), { kind: "demo" });
});

test("a cloud model names the provider that receives the conversation", () => {
  assert.deepEqual(destination("", "anthropic/claude-opus-5-5", models), { kind: "cloud", provider: "anthropic" });
  assert.equal(providerName("anthropic"), "Anthropic");
  assert.equal(providerName("vllm-lab"), "vllm-lab");
});

test("a local model stays on this computer, chosen or by default", () => {
  assert.deepEqual(destination("ollama/qwen3:32b", "anthropic/claude-opus-5-5", models), { kind: "local", provider: "ollama" });
  assert.deepEqual(destination("", "ollama/other", models), { kind: "local", provider: "ollama" });
});

test("a route or an alias is routed, not guessed", () => {
  assert.deepEqual(destination("route:fast", null, models), { kind: "routed", name: "fast" });
  assert.deepEqual(destination("", "default", models), { kind: "routed", name: "default" });
});

test("the grants say how many ask first", () => {
  assert.deepEqual(grantSummary([{ requires_approval: true }, { requires_approval: false }, { requires_approval: false }]), { granted: 3, asks: 1 });
  assert.deepEqual(grantSummary([]), { granted: 0, asks: 0 });
});
