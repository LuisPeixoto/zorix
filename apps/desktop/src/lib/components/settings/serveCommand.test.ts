import assert from "node:assert/strict";
import { test } from "node:test";
import { serveCommand } from "./serveCommand.ts";

test("by default a client only reads", () => {
  assert.equal(serveCommand({ runs: false, approvals: false, readRuns: false }), "zorix mcp serve");
});

test("each switch adds its own flag", () => {
  assert.equal(serveCommand({ runs: true, approvals: false, readRuns: false }), "zorix mcp serve --allow-runs");
  assert.equal(serveCommand({ runs: true, approvals: true, readRuns: true }), "zorix mcp serve --allow-runs --allow-approvals --allow-read-runs");
});

test("without runs the other switches add nothing", () => {
  assert.equal(serveCommand({ runs: false, approvals: true, readRuns: true }), "zorix mcp serve");
});
