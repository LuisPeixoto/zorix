import assert from "node:assert/strict";
import { test } from "node:test";

import type { Approval, ServerRow, TreeApproval } from "./api.ts";
import { approvalItems, oldestFirst, serverItems, targetOf } from "./inbox.ts";

const approval = (id: string, at: number, status: Approval["status"] = "pending", args: unknown = { path: "notes/a.md" }): Approval => ({
  id,
  execution_id: "x",
  call_id: "c",
  tool: "notes.put",
  arguments: args,
  call_hash: "h",
  reason: "It writes a note",
  status,
  requested_at: at,
  decided_at: null,
  decided_by: null,
});
const tree = (a: Approval, execution = "child", agent: string | null = "writer", depth = 1): TreeApproval => ({ execution, agent, depth, approval: a });
const run = (id: string) => ({ id, request: { prompt: `Prompt ${id}`, instructions: null, model: "demo" } });
const server = (name: string, over: Partial<ServerRow>): ServerRow =>
  ({ name, enabled: true, state: "ready", tools_to_review: 0, ...over }) as ServerRow;

test("a sub-run's approval is listed under the run the person started", () => {
  const items = approvalItems([run("r1")], { r1: [tree(approval("a1", 10))] });
  assert.equal(items.length, 1);
  assert.equal(items[0].kind === "approval" && items[0].root, "r1");
  assert.equal(items[0].kind === "approval" && items[0].execution, "child");
  assert.equal(items[0].kind === "approval" && items[0].target, "notes/a.md");
});

test("only pending approvals are listed", () => {
  const items = approvalItems([run("r1")], { r1: [tree(approval("a1", 10, "approved")), tree(approval("a2", 11))] });
  assert.deepEqual(items.map((i) => i.id), ["a2"]);
});

test("servers with tools to review or a sign-in are listed; disabled ones and quiet ones are not", () => {
  const items = serverItems([
    server("linear", { tools_to_review: 2 }),
    server("notion", { state: "needs_login" }),
    server("github", {}),
    server("old", { enabled: false, tools_to_review: 4 }),
  ]);
  assert.deepEqual(items.map((i) => i.kind === "server" && [i.server, i.why]), [["linear", "tools"], ["notion", "login"]]);
});

test("the oldest ask comes first, and servers come after the approvals", () => {
  const items = oldestFirst([
    ...serverItems([server("linear", { tools_to_review: 1 })]),
    ...approvalItems([run("r1"), run("r2")], { r1: [tree(approval("late", 50))], r2: [tree(approval("early", 5))] }),
  ]);
  assert.deepEqual(items.map((i) => i.id), ["early", "late", "server:linear"]);
});

test("the target is the first well-known argument, or the arguments in brief", () => {
  assert.equal(targetOf({ channel: "#incidents", text: "hi" }), "#incidents");
  assert.equal(targetOf({ sql: "UPDATE accounts SET plan = 1" }), "UPDATE accounts SET plan = 1");
  assert.equal(targetOf({ n: 3 }), '{"n":3}');
  assert.equal(targetOf({}), "");
  assert.equal(targetOf(null), "");
});
