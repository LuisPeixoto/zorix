// Turning the executions of one tree into what the screens draw: nested nodes, rows to show, and totals. Pure, so it is tested.
import type { Execution } from "./api.ts";

export interface TreeNode {
  run: Execution;
  children: TreeNode[];
  depth: number;
}

/**
 * The tree of `runs`. The root is the run nobody in the list started (the first one when there are several). Children keep the
 * order they were made in. A run whose parent is not in the list hangs under the root, so nothing disappears.
 */
export function buildTree(runs: Execution[]): TreeNode | null {
  if (runs.length === 0) return null;
  const ids = new Set(runs.map((r) => r.id));
  const root = runs.find((r) => !r.parent || !ids.has(r.parent)) ?? runs[0];
  const kids = new Map<string, Execution[]>();
  for (const run of runs) {
    if (run.id === root.id) continue;
    const parent = run.parent && ids.has(run.parent) && run.parent !== run.id ? run.parent : root.id;
    kids.set(parent, [...(kids.get(parent) ?? []), run]);
  }
  const seen = new Set<string>();
  const make = (run: Execution, depth: number): TreeNode => {
    seen.add(run.id);
    const children = (kids.get(run.id) ?? [])
      .filter((c) => !seen.has(c.id))
      .sort((a, b) => a.created_at - b.created_at || a.id.localeCompare(b.id))
      .map((c) => make(c, depth + 1));
    return { run, children, depth };
  };
  return make(root, 0);
}

export interface Row {
  node: TreeNode;
  depth: number;
  hasChildren: boolean;
  expanded: boolean;
}

/** The rows to show, top to bottom, leaving out what is inside a collapsed node. */
export function flatten(root: TreeNode, collapsed: ReadonlySet<string>): Row[] {
  const rows: Row[] = [];
  const walk = (node: TreeNode) => {
    const hasChildren = node.children.length > 0;
    const expanded = hasChildren && !collapsed.has(node.run.id);
    rows.push({ node, depth: node.depth, hasChildren, expanded });
    if (expanded) node.children.forEach(walk);
  };
  walk(root);
  return rows;
}

export interface Totals {
  runs: number;
  working: number;
  waiting: number;
  done: number;
  failed: number;
  cancelled: number;
  steps: number;
  cost_micros: number;
}

const WORKING = new Set(["created", "queued", "running", "waiting_for_model", "waiting_for_tool", "waiting_for_children"]);

/** What the whole tree is doing, counted over every run in it. The cost is the root's: its usage already includes what its children used. */
export function totals(root: TreeNode): Totals {
  const all: Execution[] = [];
  const walk = (n: TreeNode) => {
    all.push(n.run);
    n.children.forEach(walk);
  };
  walk(root);
  return {
    runs: all.length,
    working: all.filter((r) => WORKING.has(r.state)).length,
    waiting: all.filter((r) => r.state === "waiting_for_approval" || r.state === "paused").length,
    done: all.filter((r) => r.state === "completed").length,
    failed: all.filter((r) => r.state === "failed").length,
    cancelled: all.filter((r) => r.state === "cancelled").length,
    steps: all.reduce((n, r) => n + (r.usage?.model_calls ?? 0), 0),
    cost_micros: root.run.usage?.cost_micros ?? 0,
  };
}

/** The cost of a node's own work: what it used, minus what its children used (a parent's usage includes them). Never below 0. */
export function ownCost(node: TreeNode): number {
  const children = node.children.reduce((n, c) => n + (c.run.usage?.cost_micros ?? 0), 0);
  return Math.max(0, (node.run.usage?.cost_micros ?? 0) - children);
}

/** Each node's share of the root's cost, 0 to 1, by what it spent itself. */
export function costShares(root: TreeNode): Map<string, number> {
  const total = Math.max(1, root.run.usage?.cost_micros ?? 0);
  const out = new Map<string, number>();
  const walk = (n: TreeNode) => {
    out.set(n.run.id, Math.min(1, ownCost(n) / total));
    n.children.forEach(walk);
  };
  walk(root);
  return out;
}

/** How long a run took, in milliseconds, or null while it has not started. */
export function elapsed(run: Execution, now: number): number | null {
  if (run.started_at == null) return null;
  return Math.max(0, (run.finished_at ?? now) - run.started_at);
}

/** The id of every ancestor of `id` in the tree, nearest first (empty for the root or an unknown id). */
export function ancestors(root: TreeNode, id: string): string[] {
  const path: string[] = [];
  const find = (n: TreeNode): boolean => {
    if (n.run.id === id) return true;
    for (const c of n.children) {
      if (find(c)) {
        path.push(n.run.id);
        return true;
      }
    }
    return false;
  };
  return find(root) ? path : [];
}
