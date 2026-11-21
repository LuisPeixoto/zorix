import assert from "node:assert/strict";
import { test } from "node:test";

import type { WfRun, WfStep, WorkflowSpec } from "./api.ts";
import {
  GRID,
  NODE_H,
  NODE_W,
  addStep,
  ancestorsOf,
  barHeights,
  barTone,
  bounds,
  clampZoom,
  connect,
  describeWhen,
  detailLine,
  disconnect,
  edgeMid,
  edgePath,
  edgeState,
  emptyHistory,
  fit,
  freeSpot,
  fromUnit,
  kindLine,
  miniMap,
  ms,
  moveStep,
  reaches,
  redo,
  relabel,
  remember,
  removeStep,
  savedText,
  showOptions,
  slugify,
  snap,
  toUnit,
  triggerLabel,
  uniqueId,
  undo,
  zoomAt,
} from "./workflow.ts";

const known = { agents: ["researcher"], teams: ["research-desk"], workflows: ["weekly-report"] };

const blank = (): WorkflowSpec => ({ name: "Flow", steps: [{ id: "start", title: "Start by hand", x: 96, y: 32, kind: { type: "manual" } }], edges: [] });

function built(): WorkflowSpec {
  let spec = blank();
  for (const [kind, after] of [["agent", "start"], ["condition", "agent"], ["approval", "condition"], ["send", "approval"]] as const) {
    const added = addStep(spec, kind, known, after);
    assert.ok("spec" in added);
    spec = added.spec;
  }
  return spec;
}

test("the chip says what the Rust side says", () => {
  assert.equal(describeWhen({ type: "daily", times: ["08:00"], days: ["mon", "tue", "wed", "thu", "fri"] }), "Weekdays 08:00");
  assert.equal(describeWhen({ type: "daily", times: ["16:00"], days: ["fri"] }), "Fridays 16:00");
  assert.equal(describeWhen({ type: "daily", times: ["09:30"], days: [] }), "Daily 09:30");
  assert.equal(describeWhen({ type: "daily", times: ["09:30"], days: ["mon", "wed"] }), "Mon, Wed 09:30");
  assert.equal(describeWhen({ type: "every", minutes: 15 }), "Every 15 minutes");
  assert.equal(describeWhen({ type: "every", minutes: 120 }), "Every 2 hours");
  assert.equal(describeWhen({ type: "every", minutes: 60 }), "Every hour");
  assert.equal(describeWhen({ type: "cron", expr: "0 8 * * 1-5" }), "Cron 0 8 * * 1-5");
  assert.equal(triggerLabel({ type: "webhook", source: "GitHub", label: "issue labelled" }), "GitHub · issue labelled");
  assert.equal(triggerLabel({ type: "webhook", source: "Sentry" }), "Sentry · webhook");
  assert.equal(triggerLabel({ type: "after_workflow", workflow: "weekly-report", on: "finished" }), "After weekly-report");
  assert.equal(triggerLabel({ type: "event", on: "failed", agent: "writer" }), "When writer fails");
  assert.equal(triggerLabel({ type: "agent", agent: "a", prompt: "p" }), null);
});

test("durations use the largest whole unit and convert back", () => {
  assert.equal(ms(7_200_000), "2 h");
  assert.equal(ms(5_400_000), "90 min");
  assert.equal(ms(172_800_000), "2 d");
  assert.deepEqual(toUnit(7_200_000), { amount: 2, unit: "h" });
  assert.deepEqual(toUnit(90 * 60_000), { amount: 90, unit: "min" });
  assert.equal(fromUnit(2, "h"), 7_200_000);
  assert.equal(fromUnit(3, "d"), 259_200_000);
});

test("the lines under a step name what it uses", () => {
  const agent: WfStep = { id: "a", title: "Triage", x: 0, y: 0, kind: { type: "agent", agent: "researcher", prompt: "Look   into\nit", version: 7 } };
  assert.equal(kindLine(agent), "Agent · researcher v7");
  assert.equal(detailLine(agent), "Look into it");
  const approval: WfStep = { id: "p", title: "On-call", x: 0, y: 0, kind: { type: "approval", message: "Go?", timeout_ms: 7_200_000 } };
  assert.equal(kindLine(approval), "Approval · 2 h timeout");
  const send: WfStep = { id: "s", title: "Post", x: 0, y: 0, kind: { type: "send", service: "slack", channel: "#incidents", text: "x" } };
  assert.equal(kindLine(send), "Action · Slack");
  assert.equal(detailLine(send), "#incidents");
  assert.equal(detailLine({ ...agent, kind: { type: "agent", agent: "a", prompt: "x".repeat(80) } }).length, 30);
});

test("an edge leaves the bottom of one node and enters the top of the next, like the design system", () => {
  // The design system's first arrow: from the node at (96, 28) to the one at (228, 168).
  assert.equal(edgePath({ x: 96, y: 28 }, { x: 228, y: 168 }), "M190 96 C190 140, 322 124, 322 168");
  assert.deepEqual(edgeMid({ x: 96, y: 28 }, { x: 228, y: 168 }), { x: 256, y: 132 });
});

test("positions snap to the 16 px grid", () => {
  assert.equal(snap(0), 0);
  assert.equal(snap(7), 0);
  assert.equal(snap(9), 16);
  assert.equal(snap(-9), -16);
  assert.equal(GRID, 16);
  assert.equal(clampZoom(0.2), 0.5);
  assert.equal(clampZoom(3), 1.5);
  assert.equal(clampZoom(1.04), 1.04);
});

test("fit shows every step and zoom keeps the point under the pointer", () => {
  const steps = [{ x: 0, y: 0 }, { x: 1000, y: 800 }];
  const view = fit(steps, { w: 800, h: 600 });
  assert.ok(view.zoom >= 0.5 && view.zoom <= 1);
  const box = bounds(steps);
  // The content, scaled and moved, sits inside the viewport.
  assert.ok(box.minX * view.zoom + view.panX >= 0 && box.maxX * view.zoom + view.panX <= 800);
  assert.ok(box.minY * view.zoom + view.panY >= 0 && box.maxY * view.zoom + view.panY <= 600);
  assert.equal(fit([{ x: 0, y: 0 }], { w: 800, h: 600 }).zoom, 1, "a small workflow is not blown up");

  const start = { zoom: 1, panX: 40, panY: 20 };
  const at = { x: 300, y: 200 };
  const world = { x: (at.x - start.panX) / start.zoom, y: (at.y - start.panY) / start.zoom };
  const next = zoomAt(start, 1.5, at);
  assert.ok(Math.abs(world.x * next.zoom + next.panX - at.x) <= 1);
  assert.ok(Math.abs(world.y * next.zoom + next.panY - at.y) <= 1);
});

test("the minimap scales the steps and the visible part into its box", () => {
  const steps = [{ x: 0, y: 0 }, { x: 400, y: 400 }];
  const map = miniMap(steps, { zoom: 1, panX: 0, panY: 0 }, { w: 300, h: 300 }, { w: 120, h: 72 });
  assert.equal(map.nodes.length, 2);
  assert.ok(map.nodes.every((n) => n.x >= 0 && n.x <= 120 && n.y >= 0 && n.y <= 72));
  assert.ok(map.viewport.w > 0 && map.viewport.h > 0);
});

test("a new step gets a name nobody has and a free place below the one it follows", () => {
  assert.equal(uniqueId("agent", [{ id: "agent" }, { id: "agent-2" }]), "agent-3");
  assert.equal(uniqueId("manual", []), "manual");
  const spec = blank();
  const added = addStep(spec, "agent", known, "start");
  assert.ok("spec" in added);
  const agent = added.spec.steps[1];
  assert.equal(agent.id, "agent");
  assert.deepEqual([agent.x, agent.y], [96, 32 + NODE_H + 76 - ((32 + NODE_H + 76) % GRID)]);
  assert.equal(agent.kind.type === "agent" && agent.kind.agent, "researcher");
  // The next one under the same step does not land on top of it.
  const second = addStep(added.spec, "agent", known, "start");
  assert.ok("spec" in second);
  const [a, b] = [second.spec.steps[1], second.spec.steps[2]];
  assert.ok(Math.abs(a.x - b.x) >= NODE_W || Math.abs(a.y - b.y) >= NODE_H);
  assert.equal(freeSpot([], undefined).x, 96);
});

test("a step that needs something that does not exist says so instead of being made invalid", () => {
  const spec = blank();
  const none = addStep(spec, "after_workflow", { agents: [], teams: [], workflows: [] });
  assert.ok("error" in none);
  const some = addStep(spec, "after_workflow", known);
  assert.ok("spec" in some);
  const step = some.spec.steps[1];
  assert.equal(step.kind.type === "after_workflow" && step.kind.workflow, "weekly-report");
  const crowded: WorkflowSpec = { ...spec, steps: Array.from({ length: 50 }, (_, i) => ({ id: `s${i}`, title: "x", x: i * 300, y: 0, kind: { type: "manual" } })) };
  assert.ok("error" in addStep(crowded, "agent", known));
});

test("a condition starts from what the step before it said", () => {
  const spec = built();
  const condition = spec.steps.find((s) => s.id === "condition");
  assert.equal(condition?.kind.type === "condition" && condition.kind.expr, 'steps.agent.output.text contains "yes"');
});

test("joining steps labels a condition's arrows yes then no and refuses what cannot be", () => {
  let spec = built();
  const added = addStep(spec, "agent", known, "condition");
  assert.ok("spec" in added);
  spec = added.spec;
  const one = connect(spec, "agent", "condition");
  assert.ok("spec" in one);
  spec = one.spec;
  assert.deepEqual(spec.edges.at(-1), { from: "agent", to: "condition" });
  const yes = connect(spec, "condition", "approval");
  assert.ok("spec" in yes);
  assert.equal(yes.spec.edges.at(-1)?.label, "yes");
  const no = connect(yes.spec, "condition", "agent-2");
  assert.ok("spec" in no);
  assert.equal(no.spec.edges.at(-1)?.label, "no");
  const third = connect(no.spec, "condition", "message");
  assert.ok("error" in third && /one yes arrow and one no arrow/.test(third.error));

  assert.ok("error" in connect(spec, "agent", "agent"), "itself");
  assert.ok("error" in connect(spec, "agent", "condition"), "twice");
  assert.ok("error" in connect(spec, "agent", "start"), "into a trigger");
  const loop = connect(yes.spec, "approval", "agent");
  assert.ok("error" in loop && /loop/.test(loop.error));
  assert.equal(reaches(yes.spec, "agent", "approval"), true);
  assert.equal(reaches(yes.spec, "approval", "agent"), false);
});

test("an arrow can be removed and a condition's branches swapped", () => {
  let spec = built();
  for (const [from, to] of [["start", "agent"], ["agent", "condition"], ["condition", "approval"], ["condition", "message"]]) {
    const joined = connect(spec, from, to);
    assert.ok("spec" in joined);
    spec = joined.spec;
  }
  assert.equal(spec.edges.find((e) => e.to === "approval")?.label, "yes");
  const swapped = relabel(spec, "condition", "approval", "no");
  assert.equal(swapped.edges.find((e) => e.to === "approval")?.label, "no");
  assert.equal(swapped.edges.find((e) => e.to === "message")?.label, "yes", "the other arrow takes the branch this one gave up");
  assert.equal(disconnect(spec, "start", "agent").edges.length, spec.edges.length - 1);
});

test("removing a step removes its arrows, and the last trigger stays", () => {
  let spec = built();
  for (const [from, to] of [["start", "agent"], ["agent", "condition"]]) {
    const joined = connect(spec, from, to);
    assert.ok("spec" in joined);
    spec = joined.spec;
  }
  const gone = removeStep(spec, "agent");
  assert.ok("spec" in gone);
  assert.equal(gone.spec.steps.some((s) => s.id === "agent"), false);
  assert.equal(gone.spec.edges.length, 0);
  const last = removeStep(spec, "start");
  assert.ok("error" in last && /needs a trigger/.test(last.error));
  assert.ok("error" in removeStep(spec, "nope"));
});

test("moving a step snaps it", () => {
  const spec = moveStep(blank(), "start", { x: 133, y: 71 });
  assert.deepEqual([spec.steps[0].x, spec.steps[0].y], [128, 64]);
});

test("an approval can show what the steps before it said, what started the run and what it cost", () => {
  let spec = built();
  for (const [from, to] of [["start", "agent"], ["agent", "condition"], ["condition", "approval"]]) {
    const joined = connect(spec, from, to);
    assert.ok("spec" in joined);
    spec = joined.spec;
  }
  assert.deepEqual(ancestorsOf(spec, "approval").map((s) => s.id), ["condition", "agent", "start"]);
  assert.deepEqual(showOptions(spec, "approval").map((o) => o.path), ["steps.agent.output.text", "trigger.payload", "run.cost_micros"]);
});

test("undo and redo walk back and forth and forget the future after a new change", () => {
  let history = emptyHistory<number>();
  history = remember(history, 1);
  history = remember(history, 2);
  const back = undo(history, 3);
  assert.ok(back);
  assert.equal(back.value, 2);
  const again = undo(back.history, back.value);
  assert.ok(again);
  assert.equal(again.value, 1);
  assert.equal(undo(again.history, again.value), null);
  const forward = redo(again.history, again.value);
  assert.ok(forward);
  assert.equal(forward.value, 2);
  assert.deepEqual(remember(forward.history, 9).future, []);
  let long = emptyHistory<number>();
  for (let i = 0; i < 80; i += 1) long = remember(long, i);
  assert.equal(long.past.length, 50);
});

function runOf(states: Record<string, { state: string; note?: string }>): WfRun {
  const steps = Object.fromEntries(Object.entries(states).map(([id, s]) => [id, { cost_micros: 0, ...s }]));
  return { id: "r", slug: "x", workflow: "w", version: 1, test: false, spec: blank(), fired: { step: "start", label: "Manual", payload: {}, at: 0 }, state: "waiting", steps, cost_micros: 0, chain: 0, started_at: 0, revision: 0 } as unknown as WfRun;
}

test("an arrow in a run is moving, tinted, faint or plain", () => {
  const run = runOf({
    start: { state: "done" },
    triage: { state: "done" },
    high: { state: "done", note: "yes" },
    approve: { state: "waiting" },
    post: { state: "skipped" },
    later: { state: "pending" },
  });
  assert.equal(edgeState(run, { from: "start", to: "triage" }), "done");
  assert.equal(edgeState(run, { from: "high", to: "approve", label: "yes" }), "live");
  assert.equal(edgeState(run, { from: "high", to: "post", label: "no" }), "skip");
  assert.equal(edgeState(run, { from: "approve", to: "later" }), "idle");
  assert.equal(edgeState(run, { from: "high", to: "later", label: "no" }), "skip", "a branch not taken is faint even before its step is skipped");
  assert.equal(edgeState(run, { from: "nope", to: "later" }), "idle");
});

test("a bar is green when it finished, amber when a person was waited for, red when it failed", () => {
  assert.equal(barTone({ state: "completed", waited: false }), "ok");
  assert.equal(barTone({ state: "completed", waited: true }), "warn");
  assert.equal(barTone({ state: "waiting", waited: true }), "warn");
  assert.equal(barTone({ state: "failed", waited: false }), "err");
  assert.equal(barTone({ state: "running", waited: false }), "accent");
  assert.equal(barTone({ state: "cancelled", waited: false }), "muted");
});

test("figures and names read well", () => {
  assert.equal(savedText(45), "45 min");
  assert.equal(savedText(31 * 60), "31 h");
  assert.equal(slugify("Incident triage!"), "incident-triage");
  assert.equal(slugify("  Weekly   report — Q4 "), "weekly-report-q4");
  assert.equal(slugify("Reunião diária"), "reuniao-diaria");
  assert.equal(slugify("!!!"), "");
});

test("a bar is as tall as its run was long, never less than 40 percent", () => {
  assert.deepEqual(barHeights([{ ms: 0 }, { ms: 250 }, { ms: 1000 }]), [40, 70, 100]);
  assert.deepEqual(barHeights([]), []);
  assert.deepEqual(barHeights([{ ms: 0 }]), [40]);
});
