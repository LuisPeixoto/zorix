// What the canvas and the workflow screens compute: geometry, the rules for adding and joining steps, the state of a node in a run,
// and small readable texts. Pure, tested in workflow.test.ts. It decides nothing the Rust side does not check again on save.
import type { Day, RunStateName, StepKind, StepKindName, StepStateName, WfBar, WfEdge, WfRun, WfStep, When, WorkflowSpec } from "./api.ts";

export const NODE_W = 188;
export const NODE_H = 68;
export const GRID = 16;
export const MIN_ZOOM = 0.5;
export const MAX_ZOOM = 1.5;
export const MAX_STEPS = 50;

export const snap = (n: number): number => Math.round(n / GRID) * GRID;
export const clampZoom = (z: number): number => Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, Math.round(z * 100) / 100));

// ---- The five kinds of node, and the palette -----------------------------------------------------------------------------

export type NodeClass = "trigger" | "agent" | "logic" | "approval" | "action";
export type PaletteGroup = "Triggers" | "Work" | "Logic" | "People" | "Actions";

export interface KindInfo {
  kind: StepKindName;
  label: string;
  icon: string;
  cls: NodeClass;
  group: PaletteGroup;
  /** What a person reads under the title of a step of this kind ("Agent", "Webhook · GitHub"). */
  word: string;
}

export const KINDS: KindInfo[] = [
  { kind: "schedule", label: "Schedule", icon: "calendar", cls: "trigger", group: "Triggers", word: "Schedule" },
  { kind: "webhook", label: "Webhook", icon: "webhook", cls: "trigger", group: "Triggers", word: "Webhook" },
  { kind: "event", label: "Event", icon: "zap", cls: "trigger", group: "Triggers", word: "Event" },
  { kind: "after_workflow", label: "After workflow", icon: "workflow", cls: "trigger", group: "Triggers", word: "After workflow" },
  { kind: "manual", label: "Manual", icon: "play", cls: "trigger", group: "Triggers", word: "Manual" },
  { kind: "agent", label: "Agent", icon: "bot", cls: "agent", group: "Work", word: "Agent" },
  { kind: "team", label: "Team", icon: "users", cls: "agent", group: "Work", word: "Team" },
  { kind: "condition", label: "Condition", icon: "split", cls: "logic", group: "Logic", word: "Condition" },
  { kind: "loop", label: "Loop", icon: "refresh", cls: "logic", group: "Logic", word: "Loop" },
  { kind: "wait", label: "Wait", icon: "clock", cls: "logic", group: "Logic", word: "Wait" },
  { kind: "approval", label: "Approval", icon: "shield-check", cls: "approval", group: "People", word: "Approval" },
  { kind: "question", label: "Ask a question", icon: "message", cls: "approval", group: "People", word: "Question" },
  { kind: "send", label: "Send message", icon: "send", cls: "action", group: "Actions", word: "Action" },
  { kind: "http", label: "HTTP request", icon: "globe", cls: "action", group: "Actions", word: "Action" },
];

export const PALETTE_GROUPS: PaletteGroup[] = ["Triggers", "Work", "Logic", "People", "Actions"];

export const kindInfo = (kind: StepKindName): KindInfo => KINDS.find((k) => k.kind === kind) ?? KINDS[0];

/** Tailwind classes per node class. They are written out so the build can see them. */
export const NODE_TEXT: Record<NodeClass, string> = {
  trigger: "text-node-trigger",
  agent: "text-node-agent",
  logic: "text-node-logic",
  approval: "text-node-approval",
  action: "text-node-action",
};
export const NODE_BG: Record<NodeClass, string> = {
  trigger: "bg-node-trigger",
  agent: "bg-node-agent",
  logic: "bg-node-logic",
  approval: "bg-node-approval",
  action: "bg-node-action",
};
export const NODE_TINT: Record<NodeClass, string> = {
  trigger: "bg-node-trigger/14",
  agent: "bg-node-agent/14",
  logic: "bg-node-logic/14",
  approval: "bg-node-approval/14",
  action: "bg-node-action/14",
};

// ---- Texts ---------------------------------------------------------------------------------------------------------------

const DAY_NAMES: Record<Day, [string, string]> = {
  mon: ["Mon", "Mondays"],
  tue: ["Tue", "Tuesdays"],
  wed: ["Wed", "Wednesdays"],
  thu: ["Thu", "Thursdays"],
  fri: ["Fri", "Fridays"],
  sat: ["Sat", "Saturdays"],
  sun: ["Sun", "Sundays"],
};
export const DAYS: Day[] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
export const WEEKDAYS: Day[] = ["mon", "tue", "wed", "thu", "fri"];

/** "Weekdays 08:00", "Fridays 16:00", "Every 15 minutes": the same words the Rust side writes. */
export function describeWhen(when: When): string {
  if (when.type === "every") {
    if (when.minutes >= 60 && when.minutes % 60 === 0) return when.minutes === 60 ? "Every hour" : `Every ${when.minutes / 60} hours`;
    return `Every ${when.minutes} minutes`;
  }
  if (when.type === "cron") return `Cron ${when.expr}`;
  const set = new Set(when.days);
  let who: string;
  if (set.size === 0 || set.size === 7) who = "Daily";
  else if (set.size === 5 && WEEKDAYS.every((d) => set.has(d))) who = "Weekdays";
  else if (set.size === 1) who = DAY_NAMES[[...set][0]][1];
  else who = DAYS.filter((d) => set.has(d)).map((d) => DAY_NAMES[d][0]).join(", ");
  return `${who} ${when.times.join(", ")}`;
}

/** The text of a trigger's chip, or null for a step that is not a trigger. */
export function triggerLabel(kind: StepKind): string | null {
  switch (kind.type) {
    case "manual":
      return "Manual";
    case "schedule":
      return describeWhen(kind.when);
    case "webhook":
      return `${kind.source} · ${kind.label || "webhook"}`;
    case "event": {
      const what = kind.on === "finished" ? "finishes" : kind.on === "failed" ? "fails" : "ends";
      return kind.agent ? `When ${kind.agent} ${what}` : `When a run ${what}`;
    }
    case "after_workflow":
      return `After ${kind.workflow}`;
    default:
      return null;
  }
}

export type TriggerChipKind = "schedule" | "webhook" | "event" | "manual" | "workflow";
export const chipKind = (kind: StepKindName): TriggerChipKind => (kind === "after_workflow" ? "workflow" : (kind as TriggerChipKind));

export function ms(value: number): string {
  const minutes = Math.round(value / 60_000);
  if (minutes >= 60 * 24 && minutes % (60 * 24) === 0) return `${minutes / (60 * 24)} d`;
  if (minutes >= 60 && minutes % 60 === 0) return `${minutes / 60} h`;
  return `${minutes} min`;
}

export type Unit = "min" | "h" | "d";
export const UNIT_MS: Record<Unit, number> = { min: 60_000, h: 3_600_000, d: 86_400_000 };
export function toUnit(value: number): { amount: number; unit: Unit } {
  if (value >= UNIT_MS.d && value % UNIT_MS.d === 0) return { amount: value / UNIT_MS.d, unit: "d" };
  if (value >= UNIT_MS.h && value % UNIT_MS.h === 0) return { amount: value / UNIT_MS.h, unit: "h" };
  return { amount: Math.max(1, Math.round(value / UNIT_MS.min)), unit: "min" };
}
export const fromUnit = (amount: number, unit: Unit): number => Math.round(amount * UNIT_MS[unit]);

const clip = (text: string, n: number): string => (text.length > n ? `${text.slice(0, n - 1).trimEnd()}…` : text);
const oneLine = (text: string): string => text.replace(/\s+/g, " ").trim();

/** The line under a step's title: what kind it is, and who or what it uses. */
export function kindLine(step: WfStep): string {
  const k = step.kind;
  const word = kindInfo(k.type).word;
  switch (k.type) {
    case "webhook":
      return `${word} · ${k.source}`;
    case "agent":
    case "loop":
      return `${word} · ${k.agent}${k.type === "agent" && k.version ? ` v${k.version}` : ""}`;
    case "team":
      return `${word} · ${k.team}`;
    case "approval":
      return k.timeout_ms ? `${word} · ${ms(k.timeout_ms)} timeout` : word;
    case "send":
      return `${word} · ${k.service === "slack" ? "Slack" : "Discord"}`;
    case "http":
      return `${word} · ${k.method}`;
    default:
      return word;
  }
}

/** The small text at the bottom of a step in the editor, before there is a run to say anything. */
export function detailLine(step: WfStep): string {
  const k = step.kind;
  switch (k.type) {
    case "schedule":
      return k.timezone ?? "";
    case "webhook":
      return k.filter ? clip(oneLine(k.filter), 28) : "any request";
    case "event":
      return k.agent ?? "any agent or team";
    case "after_workflow":
      return k.workflow;
    case "agent":
    case "team":
    case "loop":
      return clip(oneLine(k.prompt), 30);
    case "condition":
      return clip(oneLine(k.expr), 30);
    case "wait":
      return k.for_ms ? ms(k.for_ms) : `until ${k.until}`;
    case "approval":
      return clip(oneLine(k.message), 30);
    case "question":
      return clip(oneLine(k.question), 30);
    case "send":
      return k.channel;
    case "http":
      return clip(k.url.replace(/^https?:\/\//, ""), 30);
    default:
      return "";
  }
}

// ---- Geometry ------------------------------------------------------------------------------------------------------------

export interface Pt {
  x: number;
  y: number;
}

/** The cubic curve from the bottom port of `a` to the top port of `b`, as the design system draws it. */
export function edgePath(a: Pt, b: Pt): string {
  const x1 = a.x + NODE_W / 2;
  const y1 = a.y + NODE_H;
  const x2 = b.x + NODE_W / 2;
  const y2 = b.y;
  return `M${x1} ${y1} C${x1} ${y1 + 44}, ${x2} ${y2 - 44}, ${x2} ${y2}`;
}

/** Where the label of an edge sits: the middle of the curve. */
export function edgeMid(a: Pt, b: Pt): Pt {
  return { x: (a.x + b.x + NODE_W) / 2, y: (a.y + NODE_H + b.y) / 2 };
}

export interface Box {
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
}

export function bounds(steps: readonly Pt[]): Box {
  if (steps.length === 0) return { minX: 0, minY: 0, maxX: NODE_W, maxY: NODE_H };
  return {
    minX: Math.min(...steps.map((s) => s.x)),
    minY: Math.min(...steps.map((s) => s.y)),
    maxX: Math.max(...steps.map((s) => s.x)) + NODE_W,
    maxY: Math.max(...steps.map((s) => s.y)) + NODE_H,
  };
}

export interface View {
  zoom: number;
  panX: number;
  panY: number;
}

/** A view that shows every step with some air around, within the zoom limits. */
export function fit(steps: readonly Pt[], size: { w: number; h: number }, pad = 48): View {
  const box = bounds(steps);
  const w = box.maxX - box.minX;
  const h = box.maxY - box.minY;
  const zoom = clampZoom(Math.min((size.w - pad * 2) / w, (size.h - pad * 2) / h, 1));
  return { zoom, panX: Math.round((size.w - w * zoom) / 2 - box.minX * zoom), panY: Math.round((size.h - h * zoom) / 2 - box.minY * zoom) };
}

/** Zoom around a point of the viewport, so what is under the pointer stays under it. */
export function zoomAt(view: View, next: number, at: Pt): View {
  const zoom = clampZoom(next);
  const wx = (at.x - view.panX) / view.zoom;
  const wy = (at.y - view.panY) / view.zoom;
  return { zoom, panX: Math.round(at.x - wx * zoom), panY: Math.round(at.y - wy * zoom) };
}

export interface MiniMap {
  nodes: { x: number; y: number; w: number }[];
  viewport: { x: number; y: number; w: number; h: number };
}

/** The steps and the visible part, scaled into a box of `box.w` by `box.h` (with `pad` inside). */
export function miniMap(steps: readonly Pt[], view: View, size: { w: number; h: number }, box: { w: number; h: number }, pad = 8): MiniMap {
  const visible: Box = { minX: -view.panX / view.zoom, minY: -view.panY / view.zoom, maxX: (size.w - view.panX) / view.zoom, maxY: (size.h - view.panY) / view.zoom };
  const all = bounds(steps);
  const world: Box = { minX: Math.min(all.minX, visible.minX), minY: Math.min(all.minY, visible.minY), maxX: Math.max(all.maxX, visible.maxX), maxY: Math.max(all.maxY, visible.maxY) };
  const scale = Math.min((box.w - pad * 2) / (world.maxX - world.minX), (box.h - pad * 2) / (world.maxY - world.minY));
  const at = (p: Pt): Pt => ({ x: Math.round(pad + (p.x - world.minX) * scale), y: Math.round(pad + (p.y - world.minY) * scale) });
  const corner = at({ x: visible.minX, y: visible.minY });
  return {
    nodes: steps.map((s) => ({ ...at(s), w: Math.max(4, Math.round(NODE_W * scale)) })),
    viewport: { ...corner, w: Math.max(6, Math.round((visible.maxX - visible.minX) * scale)), h: Math.max(6, Math.round((visible.maxY - visible.minY) * scale)) },
  };
}

// ---- Editing -------------------------------------------------------------------------------------------------------------

/** A name for a new step that no step has: `agent`, `agent-2`, `agent-3`… */
export function uniqueId(base: string, steps: readonly { id: string }[]): string {
  const taken = new Set(steps.map((s) => s.id));
  const stem = base.replace(/_/g, "-");
  if (!taken.has(stem)) return stem;
  for (let n = 2; ; n += 1) if (!taken.has(`${stem}-${n}`)) return `${stem}-${n}`;
}

export interface Known {
  agents: string[];
  teams: string[];
  workflows: string[];
}

const overlaps = (a: Pt, b: Pt): boolean => Math.abs(a.x - b.x) < NODE_W + GRID && Math.abs(a.y - b.y) < NODE_H + GRID;

/** A free place for a new step: below the step it follows, or in the first free row of the first column. */
export function freeSpot(steps: readonly Pt[], after?: Pt): Pt {
  let spot: Pt = after ? { x: after.x, y: snap(after.y + NODE_H + 76) } : { x: snap(96), y: snap(32) };
  let guard = 0;
  while (steps.some((s) => overlaps(s, spot)) && guard < 200) {
    spot = after ? { x: snap(spot.x + NODE_W + 48), y: spot.y } : { x: spot.x, y: snap(spot.y + NODE_H + 76) };
    if (after && guard % 3 === 2) spot = { x: after.x, y: snap(spot.y + NODE_H + 76) };
    guard += 1;
  }
  return spot;
}

const TITLES: Record<StepKindName, string> = {
  manual: "Start by hand",
  schedule: "Every weekday at 08:00",
  webhook: "A request arrives",
  event: "A run ends",
  after_workflow: "Another workflow ends",
  agent: "Ask the agent",
  team: "Ask the team",
  condition: "Is it a yes?",
  loop: "For each item",
  wait: "Wait a while",
  approval: "A person approves",
  question: "Ask a question",
  send: "Send a message",
  http: "Call a web address",
};

/** The settings a new step of a kind starts with: valid, and easy to change. */
export function defaultKind(kind: StepKindName, known: Known, previous?: string): StepKind {
  switch (kind) {
    case "manual":
      return { type: "manual" };
    case "schedule":
      return { type: "schedule", when: { type: "daily", times: ["08:00"], days: [...WEEKDAYS] }, timezone: null };
    case "webhook":
      return { type: "webhook", source: "GitHub", label: "event", filter: null, signed: false };
    case "event":
      return { type: "event", on: "finished", agent: null };
    case "after_workflow":
      return { type: "after_workflow", workflow: known.workflows[0] ?? "", on: "finished" };
    case "agent":
      return { type: "agent", agent: known.agents[0] ?? "researcher", prompt: "Describe what this step should do." };
    case "team":
      return { type: "team", team: known.teams[0] ?? "research-desk", prompt: "Describe what this step should do." };
    case "condition":
      return { type: "condition", expr: previous ? `steps.${previous}.output.text contains "yes"` : "true" };
    case "loop":
      return { type: "loop", over: "{{trigger.payload.items}}", agent: known.agents[0] ?? "researcher", prompt: "Check {{item}}." };
    case "wait":
      return { type: "wait", for_ms: 10 * UNIT_MS.min };
    case "approval":
      return { type: "approval", message: "Go ahead?", show: [], timeout_ms: 2 * UNIT_MS.h, then: "refuse" };
    case "question":
      return { type: "question", question: "What should I know?", then: "refuse" };
    case "send":
      return { type: "send", service: "slack", channel: "#general", text: "Hello from ZORIX.", ask: "always" };
    case "http":
      return { type: "http", method: "POST", url: "https://example.com/hook", ask: "always" };
  }
}

/** Add a step of a kind, below `after` when given. The spec itself is not changed. */
export function addStep(spec: WorkflowSpec, kind: StepKindName, known: Known, after?: string, at?: Pt): { spec: WorkflowSpec; id: string } | { error: string } {
  if (spec.steps.length >= MAX_STEPS) return { error: `A workflow has ${MAX_STEPS} steps at most.` };
  if (kind === "after_workflow" && known.workflows.length === 0) return { error: "There is no other workflow to follow yet." };
  const anchor = after ? spec.steps.find((s) => s.id === after) : undefined;
  const id = uniqueId(kind === "send" ? "message" : kind === "http" ? "request" : kind, spec.steps);
  const prior = anchor && !kindIsTrigger(anchor.kind.type) ? anchor.id : undefined;
  const step: WfStep = { id, title: TITLES[kind], ...(at ? { x: snap(at.x), y: snap(at.y) } : freeSpot(spec.steps, anchor)), kind: defaultKind(kind, known, prior) };
  return { spec: { ...spec, steps: [...spec.steps, step] }, id };
}

export const kindIsTrigger = (kind: StepKindName): boolean => kindInfo(kind).cls === "trigger";

/** Whether `to` can already reach `from`, which would make joining them a loop. */
export function reaches(spec: WorkflowSpec, from: string, to: string): boolean {
  const seen = new Set<string>();
  const frontier = [from];
  while (frontier.length) {
    const here = frontier.pop() as string;
    if (here === to) return true;
    if (seen.has(here)) continue;
    seen.add(here);
    for (const e of spec.edges) if (e.from === here) frontier.push(e.to);
  }
  return false;
}

/** Join two steps. A condition's arrows are labelled yes then no. Says why when it cannot. */
export function connect(spec: WorkflowSpec, from: string, to: string): { spec: WorkflowSpec } | { error: string } {
  const a = spec.steps.find((s) => s.id === from);
  const b = spec.steps.find((s) => s.id === to);
  if (!a || !b) return { error: "That step is gone." };
  if (from === to) return { error: "A step cannot lead into itself." };
  if (kindIsTrigger(b.kind.type)) return { error: "Nothing leads into a trigger: it is how the workflow starts." };
  if (spec.edges.some((e) => e.from === from && e.to === to)) return { error: "Those two are already joined." };
  if (reaches(spec, to, from)) return { error: "That would make a loop. A workflow goes one way; use a loop step to repeat." };
  let label: string | null = null;
  if (a.kind.type === "condition") {
    const used = spec.edges.filter((e) => e.from === from).map((e) => e.label);
    label = !used.includes("yes") ? "yes" : !used.includes("no") ? "no" : null;
    if (!label) return { error: "A condition has one yes arrow and one no arrow." };
  }
  const edge: WfEdge = label ? { from, to, label } : { from, to };
  return { spec: { ...spec, edges: [...spec.edges, edge] } };
}

export function disconnect(spec: WorkflowSpec, from: string, to: string): WorkflowSpec {
  return { ...spec, edges: spec.edges.filter((e) => !(e.from === from && e.to === to)) };
}

/** Swap which branch an arrow of a condition takes. */
export function relabel(spec: WorkflowSpec, from: string, to: string, label: "yes" | "no"): WorkflowSpec {
  return {
    ...spec,
    edges: spec.edges.map((e) => {
      if (e.from !== from) return e;
      if (e.to === to) return { ...e, label };
      if (e.label === label) return { ...e, label: label === "yes" ? "no" : "yes" };
      return e;
    }),
  };
}

/** Remove a step and every arrow that touches it. The last trigger stays: a workflow needs one. */
export function removeStep(spec: WorkflowSpec, id: string): { spec: WorkflowSpec } | { error: string } {
  const step = spec.steps.find((s) => s.id === id);
  if (!step) return { error: "That step is gone." };
  if (kindIsTrigger(step.kind.type) && spec.steps.filter((s) => kindIsTrigger(s.kind.type)).length === 1) {
    return { error: "A workflow needs a trigger. Add another before removing this one." };
  }
  return { spec: { ...spec, steps: spec.steps.filter((s) => s.id !== id), edges: spec.edges.filter((e) => e.from !== id && e.to !== id) } };
}

export function moveStep(spec: WorkflowSpec, id: string, to: Pt): WorkflowSpec {
  return { ...spec, steps: spec.steps.map((s) => (s.id === id ? { ...s, x: snap(to.x), y: snap(to.y) } : s)) };
}

export function patchStep(spec: WorkflowSpec, id: string, patch: Partial<Pick<WfStep, "title">> & { kind?: StepKind }): WorkflowSpec {
  return { ...spec, steps: spec.steps.map((s) => (s.id === id ? { ...s, ...patch } : s)) };
}

/** Every step that runs before `id`, in the order a person would read them (nearest first). */
export function ancestorsOf(spec: WorkflowSpec, id: string): WfStep[] {
  const found: string[] = [];
  const frontier = [id];
  while (frontier.length) {
    const here = frontier.shift() as string;
    for (const e of spec.edges) {
      if (e.to === here && !found.includes(e.from)) {
        found.push(e.from);
        frontier.push(e.from);
      }
    }
  }
  return found.map((f) => spec.steps.find((s) => s.id === f)).filter((s): s is WfStep => Boolean(s));
}

/** The values an approval may show a person: what earlier steps wrote, what started the run, what it cost so far. */
export function showOptions(spec: WorkflowSpec, id: string): { path: string; label: string }[] {
  const out: { path: string; label: string }[] = [];
  for (const step of ancestorsOf(spec, id)) {
    const t = step.kind.type;
    if (t === "agent" || t === "team") out.push({ path: `steps.${step.id}.output.text`, label: step.title });
    else if (t === "question") out.push({ path: `steps.${step.id}.output.answer`, label: step.title });
  }
  out.push({ path: "trigger.payload", label: "What started it" });
  out.push({ path: "run.cost_micros", label: "Cost so far" });
  return out;
}

// ---- Undo ----------------------------------------------------------------------------------------------------------------

export interface History<T> {
  past: T[];
  future: T[];
}

export const emptyHistory = <T>(): History<T> => ({ past: [], future: [] });

/** Remember `value` (the state before a change). Keeps the last 50. */
export function remember<T>(history: History<T>, value: T): History<T> {
  return { past: [...history.past, value].slice(-50), future: [] };
}

export function undo<T>(history: History<T>, current: T): { history: History<T>; value: T } | null {
  const value = history.past[history.past.length - 1];
  if (value === undefined) return null;
  return { value, history: { past: history.past.slice(0, -1), future: [current, ...history.future] } };
}

export function redo<T>(history: History<T>, current: T): { history: History<T>; value: T } | null {
  const [value, ...rest] = history.future;
  if (value === undefined) return null;
  return { value, history: { past: [...history.past, current], future: rest } };
}

// ---- Runs on the canvas --------------------------------------------------------------------------------------------------

export type NodeState = "idle" | "running" | "waiting" | "done" | "failed" | "skipped";
export type EdgeState = "idle" | "live" | "done" | "skip";

export const nodeState = (state: StepStateName | undefined): NodeState => (!state || state === "pending" ? "idle" : state);

/** How an arrow looks in a run: moving into a step that is going, tinted after it, faint when its branch was not taken. */
export function edgeState(run: WfRun, edge: WfEdge): EdgeState {
  const from = run.steps[edge.from];
  const to = run.steps[edge.to];
  if (!from || !to) return "idle";
  if (from.state === "skipped" || to.state === "skipped") return "skip";
  if (from.state !== "done") return "idle";
  if (edge.label && from.note !== edge.label) return "skip";
  if (to.state === "running" || to.state === "waiting") return "live";
  if (to.state === "done" || to.state === "failed") return "done";
  return "idle";
}

export type BarTone = "ok" | "warn" | "err" | "accent" | "muted";

/** One bar of "last 14 runs": finished well, waited for a person, failed, going, or cancelled. */
export function barTone(bar: Pick<WfBar, "state" | "waited">): BarTone {
  if (bar.state === "failed") return "err";
  if (bar.state === "waiting" || (bar.state === "completed" && bar.waited)) return "warn";
  if (bar.state === "running") return "accent";
  if (bar.state === "cancelled") return "muted";
  return "ok";
}

/** The height of each bar, 40 to 100 percent, by how long the run took against the longest of the row (square root, so short runs stay visible). */
export function barHeights(bars: readonly Pick<WfBar, "ms">[]): number[] {
  const longest = Math.max(1, ...bars.map((b) => b.ms));
  return bars.map((b) => Math.round(40 + 60 * Math.sqrt(b.ms / longest)));
}

export const RUN_STATE_LABEL: Record<RunStateName, string> = {
  running: "Running",
  waiting: "Waiting for a person",
  completed: "Finished",
  failed: "Failed",
  cancelled: "Cancelled",
};

/** "minutes saved" as the figure under Time saved: `31 h`, `45 min`. */
export function savedText(minutes: number): string {
  return minutes >= 120 ? `${Math.round(minutes / 60)} h` : `${minutes} min`;
}

/** A workflow name made into a short name: `Incident triage!` becomes `incident-triage`. */
export function slugify(name: string): string {
  return name
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 48)
    .replace(/-+$/g, "");
}
