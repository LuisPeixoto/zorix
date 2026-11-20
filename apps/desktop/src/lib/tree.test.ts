import assert from "node:assert/strict";
import { test } from "node:test";

import type { Execution } from "./api.ts";
import { ancestors, buildTree, costShares, elapsed, flatten, ownCost, totals } from "./tree.ts";

function run(id: string, parent: string | null, state: string, cost: number, at: number, extra: Partial<Execution> = {}): Execution {
  return {
    id,
    parent,
    root: parent ? "root" : null,
    state,
    created_at: at,
    started_at: at,
    finished_at: state === "completed" ? at + 1000 : null,
    usage: { input_tokens: 1, output_tokens: 1, cost_micros: cost, model_calls: 2, tool_calls: 0 },
    request: { prompt: id, instructions: null, model: "m" },
    ...extra,
  } as unknown as Execution;
}

const sample = () => [
  run("root", null, "waiting_for_children", 500, 1),
  run("b", "root", "running", 100, 3),
  run("a", "root", "completed", 300, 2),
  run("a1", "a", "completed", 100, 4),
];

test("a tree is built from parent links, in the order things were made, whatever order the list came in", () => {
  const tree = buildTree(sample())!;
  assert.equal(tree.run.id, "root");
  assert.deepEqual(tree.children.map((c) => c.run.id), ["a", "b"]);
  assert.deepEqual(tree.children[0].children.map((c) => [c.run.id, c.depth]), [["a1", 2]]);
  const shuffled = buildTree([...sample()].reverse())!;
  assert.equal(shuffled.run.id, "root", "the root is the one nobody started");
  assert.equal(buildTree([]), null);
});

test("a run whose parent is missing, or that points at itself, still shows up under the root", () => {
  const lost = buildTree([run("root", null, "running", 0, 1), run("x", "gone", "running", 0, 2), run("y", "y", "running", 0, 3)])!;
  assert.deepEqual(lost.children.map((c) => c.run.id).sort(), ["x", "y"]);
});

test("a collapsed node hides what is inside it and rows say how deep they are", () => {
  const tree = buildTree(sample())!;
  assert.deepEqual(flatten(tree, new Set()).map((r) => [r.node.run.id, r.depth, r.hasChildren, r.expanded]), [
    ["root", 0, true, true],
    ["a", 1, true, true],
    ["a1", 2, false, false],
    ["b", 1, false, false],
  ]);
  assert.deepEqual(flatten(tree, new Set(["a"])).map((r) => r.node.run.id), ["root", "a", "b"]);
  assert.deepEqual(flatten(tree, new Set(["root"])).map((r) => r.node.run.id), ["root"]);
});

test("totals count every run and the cost is the root's, which already includes the others", () => {
  const t = totals(buildTree(sample())!);
  assert.deepEqual([t.runs, t.working, t.done, t.failed, t.waiting, t.steps, t.cost_micros], [4, 2, 2, 0, 0, 8, 500]);
  const mixed = totals(buildTree([run("root", null, "waiting_for_children", 9, 1), run("c", "root", "waiting_for_approval", 1, 2), run("d", "root", "failed", 1, 3), run("e", "root", "cancelled", 1, 4)])!);
  assert.deepEqual([mixed.waiting, mixed.failed, mixed.cancelled, mixed.working], [1, 1, 1, 1]);
});

test("each run's own cost is what it used minus what its children used, and the shares add up to at most the whole", () => {
  const tree = buildTree(sample())!;
  assert.equal(ownCost(tree), 100, "500 minus the 300 and 100 of its children");
  assert.equal(ownCost(tree.children[0]), 200, "300 minus the 100 of a1");
  const shares = costShares(tree);
  assert.equal(shares.get("root"), 0.2);
  assert.equal([...shares.values()].reduce((a, b) => a + b, 0), 1);
  assert.equal(ownCost(buildTree([run("r", null, "completed", 10, 1), run("k", "r", "completed", 50, 2)])!), 0, "never below zero");
});

test("elapsed time runs to now while a run works, and ancestors are listed nearest first", () => {
  const r = run("r", null, "running", 0, 1, { started_at: 1_000, finished_at: null });
  assert.equal(elapsed(r, 4_000), 3_000);
  assert.equal(elapsed(run("r", null, "completed", 0, 1, { started_at: 1_000, finished_at: 2_500 }), 9_000), 1_500);
  assert.equal(elapsed(run("r", null, "queued", 0, 1, { started_at: null }), 9_000), null);
  const tree = buildTree(sample())!;
  assert.deepEqual(ancestors(tree, "a1"), ["a", "root"]);
  assert.deepEqual(ancestors(tree, "root"), []);
  assert.deepEqual(ancestors(tree, "nope"), []);
});
