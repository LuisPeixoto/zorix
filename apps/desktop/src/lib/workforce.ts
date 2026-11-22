// The workforce: the pure rules the screens use. What decides anything (levels, budgets, the organisation's rules, figures) lives in
// Rust; this file turns it into words, numbers and the layout of the chart. Tested in workforce.test.ts.
import type { ActivityLine, AutonomyLevel, Figures, Notice, Overview, WorkerView } from "./api.ts";
import { usd } from "./shell.ts";
import type { UiTone } from "./ui/tone.ts";

// ---------------------------------------------------------------------------------------------------------------------
// Levels

export interface LevelInfo {
  id: AutonomyLevel;
  label: string;
  /** What the level means, in a sentence. */
  blurb: string;
  tone: UiTone;
}

export const LEVELS: LevelInfo[] = [
  { id: "suggest", label: "Suggest", blurb: "Proposes; a person acts. Nothing it wants to change runs.", tone: "muted" },
  { id: "ask", label: "Ask", blurb: "Acts after each approval. Every change asks first.", tone: "info" },
  { id: "notify", label: "Notify", blurb: "Acts, then tells you. What only the default would ask runs on its own.", tone: "accent" },
  { id: "budgeted", label: "Budgeted", blurb: "Acts within its weekly budget and its time for a task.", tone: "accent" },
  { id: "full", label: "Full", blurb: "Acts and reports every week.", tone: "warn" },
];

export const levelIndex = (level: AutonomyLevel): number => LEVELS.findIndex((l) => l.id === level);
export const levelLabel = (level: AutonomyLevel): string => LEVELS[levelIndex(level)]?.label ?? level;

/** The levels a worker may have under a manager: none above the manager's. With no manager (you), any. */
export function levelsUnder(managerLevel: AutonomyLevel | null): AutonomyLevel[] {
  const top = managerLevel === null ? LEVELS.length - 1 : levelIndex(managerLevel);
  return LEVELS.filter((_, i) => i <= top).map((l) => l.id);
}

// ---------------------------------------------------------------------------------------------------------------------
// Words

/** `2 h`, `45 min`, `1 h 30 min`. */
export function duration(ms: number): string {
  const minutes = Math.round(ms / 60_000);
  if (minutes < 60) return `${minutes} min`;
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return m === 0 ? `${h} h` : `${h} h ${m} min`;
}

/** Whole dollars when they are whole (`$25`), else cents (`$12.50`), as the design system writes a budget. */
export function money(micros: number): string {
  const dollars = micros / 1_000_000;
  return Number.isInteger(dollars) ? `$${dollars}` : usd(micros);
}

/** `repo:acme/web:write` → `acme/web`. Anything else is shown as it is. */
export function stillAskName(entry: string): string {
  const withoutWrite = entry.endsWith(":write") ? entry.slice(0, -":write".length) : entry;
  const at = withoutWrite.indexOf(":");
  const family = at < 0 ? "" : withoutWrite.slice(0, at);
  const rest = at < 0 ? withoutWrite : withoutWrite.slice(at + 1);
  return ["repo", "db", "jira", "http", "chat", "project"].includes(family) ? rest : entry;
}

/** The sentence under the dial: what the level lets the worker do, within what limits, and what always asks. */
export function limitsSentence(w: Pick<WorkerView["worker"], "autonomy" | "weekly_budget_micros" | "task_time_ms" | "still_ask">): string {
  const limits = [w.weekly_budget_micros !== null ? `${money(w.weekly_budget_micros)} a week` : null, `${duration(w.task_time_ms)} a task`].filter(Boolean).join(" and ");
  let first: string;
  switch (w.autonomy) {
    case "suggest":
      first = "Proposes; you act. Nothing it wants to change runs.";
      break;
    case "ask":
      first = `Acts after each approval, within ${limits}.`;
      break;
    case "notify":
      first = `Acts, then tells you, within ${limits}.`;
      break;
    case "budgeted":
      first = `Acts within ${limits}.`;
      break;
    default:
      first = `Acts and reports weekly, within ${limits}.`;
  }
  if (w.still_ask.length === 0) return first;
  const names = w.still_ask.map(stillAskName);
  const what = names.length === 1 ? names[0] : `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
  return `${first} Writes to ${what} still ask.`;
}

// ---------------------------------------------------------------------------------------------------------------------
// Figures

export function onTimeText(done: number, all: number): string {
  return all === 0 ? "—" : `${Math.round((done / all) * 100)}%`;
}

export const evalText = (score: number | null): string => (score === null ? "—" : score.toFixed(2));

export type Figure = [value: string, label: string];

/** The three figures a card shows: a manager's on-time rate, what waits for you, else the eval score; a worker with none says so. */
export function cardFigures(v: Pick<WorkerView, "figures" | "worker" | "own" | "reports">): Figure[] {
  const f = v.figures;
  const tasks: Figure = [String(f.tasks), f.tasks === 1 ? "task" : "tasks"];
  const week: Figure = [money(f.week_micros), "week"];
  if (v.worker.manager) return [tasks, [onTimeText(f.on_time_done, f.on_time_all), "on time"], week];
  if (f.waiting > 0) return [tasks, [String(f.waiting), "waiting"], week];
  if (v.own.eval !== null && v.reports.length === 0) return [tasks, [evalText(v.own.eval), "eval"], week];
  return [tasks, [onTimeText(f.on_time_done, f.on_time_all), "on time"], week];
}

/** More spend reads `warn` and less reads `ok`, whatever the direction: the change is toned by whether it is good. */
export function spendDelta(now: number, before: number): { text: string; tone: UiTone } | null {
  if (before <= 0) return null;
  const change = Math.round(((now - before) / before) * 100);
  if (change === 0) return null;
  return { text: `${change > 0 ? "▲" : "▼"} ${Math.abs(change)}%`, tone: change > 0 ? "warn" : "ok" };
}

export const STATUS_TEXT: Record<string, string> = { ok: "Working", warn: "Waiting for you", err: "Stopped" };

/** Who a worker's weekly budget is nearly or fully used up for: a share from 0 to 1, for the bar. */
export function budgetShare(figures: Pick<Figures, "week_micros">, budget: number | null): number | null {
  return budget === null || budget <= 0 ? null : Math.min(1, figures.week_micros / budget);
}

// ---------------------------------------------------------------------------------------------------------------------
// The chart

export const CARD_W = 196;
export const CARD_H = 120;
export const CHIP_W = 128;
export const CHIP_H = 30;
export const GAP_X = 14;
export const GAP_Y = 40;
const BUS = 20;
const CHIP_GAP = 8;

export interface ChartNode {
  id: string;
  parent: string | null;
}

export interface Placed {
  id: string;
  x: number;
  y: number;
  w: number;
  h: number;
  depth: number;
  /** A small chip instead of a card: the fourth level down and below. */
  chip: boolean;
}

export interface Chart {
  nodes: Placed[];
  /** SVG path data for the elbow connectors, each from a parent's bottom to a child's top. */
  paths: string[];
  width: number;
  height: number;
}

/** Cards for the first three levels (you, then who reports to you, then who reports to them), chips for everyone below. */
export function layoutChart(input: ChartNode[]): Chart {
  const children = new Map<string | null, string[]>();
  for (const n of input) children.set(n.parent, [...(children.get(n.parent) ?? []), n.id]);
  const roots = children.get(null) ?? [];
  const size = (depth: number) => (depth >= 3 ? { w: CHIP_W, h: CHIP_H, chip: true } : { w: CARD_W, h: CARD_H, chip: false });
  const widths = new Map<string, number>();
  const measure = (id: string, depth: number, seen: Set<string>): number => {
    if (seen.has(id)) return 0;
    seen.add(id);
    const kids = children.get(id) ?? [];
    const own = size(depth).w;
    const gap = depth + 1 >= 3 ? CHIP_GAP : GAP_X;
    const inner = kids.reduce((sum, k, i) => sum + measure(k, depth + 1, seen) + (i > 0 ? gap : 0), 0);
    const width = Math.max(own, inner);
    widths.set(id, width);
    return width;
  };
  const nodes: Placed[] = [];
  const paths: string[] = [];
  const rows: number[] = [];
  const rowTop = (depth: number): number => {
    if (rows[depth] !== undefined) return rows[depth];
    let y = 0;
    for (let d = 0; d < depth; d++) y += size(d).h + (d + 1 >= 3 ? 28 : GAP_Y);
    rows[depth] = y;
    return y;
  };
  const place = (id: string, depth: number, left: number, seen: Set<string>): void => {
    if (seen.has(id)) return;
    seen.add(id);
    const width = widths.get(id) ?? size(depth).w;
    const me = size(depth);
    const x = left + (width - me.w) / 2;
    const y = rowTop(depth);
    nodes.push({ id, x, y, depth, ...me });
    const kids = children.get(id) ?? [];
    const gap = depth + 1 >= 3 ? CHIP_GAP : GAP_X;
    const total = kids.reduce((sum, k, i) => sum + (widths.get(k) ?? 0) + (i > 0 ? gap : 0), 0);
    let cursor = left + (width - total) / 2;
    const centres: number[] = [];
    for (const kid of kids) {
      const kidWidth = widths.get(kid) ?? 0;
      const kidSize = size(depth + 1);
      centres.push(cursor + (kidWidth - kidSize.w) / 2 + kidSize.w / 2);
      place(kid, depth + 1, cursor, seen);
      cursor += kidWidth + gap;
    }
    if (kids.length > 0) {
      const from = { x: x + me.w / 2, y: y + me.h };
      const bus = from.y + (depth + 1 >= 3 ? 14 : BUS);
      const top = rowTop(depth + 1);
      const parts = [`M${from.x} ${from.y} V${bus}`];
      if (centres.length > 1) parts.push(`M${Math.min(...centres)} ${bus} H${Math.max(...centres)}`);
      for (const c of centres) parts.push(`M${c} ${bus} V${top}`);
      paths.push(parts.join(" "));
    }
  };
  const gapRoot = GAP_X;
  const measured = new Set<string>();
  const totalRoots = roots.reduce((sum, r, i) => sum + measure(r, 0, measured) + (i > 0 ? gapRoot : 0), 0);
  let cursor = 0;
  const placed = new Set<string>();
  for (const r of roots) {
    place(r, 0, cursor, placed);
    cursor += (widths.get(r) ?? 0) + gapRoot;
  }
  const height = nodes.reduce((max, n) => Math.max(max, n.y + n.h), 0);
  return { nodes, paths, width: totalRoots, height };
}

/** The scale that makes a chart of this width fit the room it has, never above 1. */
export const fitScale = (width: number, room: number): number => (width <= 0 || room <= 0 ? 1 : Math.min(1, room / width));

// ---------------------------------------------------------------------------------------------------------------------
// Activity and notices

export const LEVEL_TONE: Record<string, UiTone> = { OK: "ok", INFO: "info", WAIT: "warn", WARN: "warn", ERR: "err" };

export type LevelFilter = "all" | "warn";

/** `warn and above`: waiting, warnings and errors. */
export function filterLines(lines: readonly ActivityLine[], filter: LevelFilter): ActivityLine[] {
  return filter === "all" ? [...lines] : lines.filter((l) => l.level === "WAIT" || l.level === "WARN" || l.level === "ERR");
}

/** The time of a line: `14:02:11` today, else the weekday and the time. */
export function lineTime(at: number, now: number): string {
  const d = new Date(at);
  const two = (n: number) => String(n).padStart(2, "0");
  const time = `${two(d.getHours())}:${two(d.getMinutes())}:${two(d.getSeconds())}`;
  const start = new Date(now);
  start.setHours(0, 0, 0, 0);
  if (at >= start.getTime()) return time;
  const day = d.toLocaleDateString("en-US", { weekday: "short" });
  return `${day} ${time.slice(0, 5)}`;
}

/** What the Inbox says about a notice, as the sentence under its title. */
export const NOTICE_ICON: Record<Notice["kind"], string> = { suggestion: "sparkles", notice: "alert", report: "book" };

// ---------------------------------------------------------------------------------------------------------------------
// The chart's nodes

export interface OrgNode {
  id: string;
  parent: string | null;
  name: string;
  role: string;
  emoji: string;
  color: string;
  status: "ok" | "warn" | "err" | null;
  human: boolean;
  figures: Figure[];
  label: string;
}

/** The id of the card of you, the root of every chain. */
export const YOU = "you";

/** You at the top, then each worker under who it reports to (under you when it reports to nobody). */
export function orgNodes(overview: Pick<Overview, "workers" | "you"> & { settings?: Pick<Overview["settings"], "departments"> }): OrgNode[] {
  const department = (slug: string) => overview.settings?.departments.find((d) => d.slug === slug)?.name ?? slug;
  const you = overview.you;
  const nodes: OrgNode[] = [
    {
      id: YOU,
      parent: null,
      name: "You",
      role: "Owner · approves",
      emoji: "",
      color: "muted",
      status: null,
      human: true,
      figures: [
        [String(you.to_review), "to review"],
        [String(you.approvals), "approvals"],
        ["—", ""],
      ],
      label: `You, owner. ${you.to_review} to review, ${you.approvals} approvals waiting`,
    },
  ];
  for (const v of overview.workers) {
    const figures = cardFigures(v);
    nodes.push({
      id: v.worker.agent,
      parent: v.worker.reports_to ?? YOU,
      name: v.name,
      role: v.worker.manager ? `Virtual manager · ${department(v.worker.department)}` : v.worker.role,
      emoji: v.avatar.emoji,
      color: v.avatar.color,
      status: v.status,
      human: false,
      figures,
      label: `${v.name}, ${v.worker.role}, ${levelLabel(v.worker.autonomy)}${v.status ? `, ${STATUS_TEXT[v.status].toLowerCase()}` : ""}. ${figures.map(([value, caption]) => `${value} ${caption}`).join(", ")}`,
    });
  }
  return nodes;
}
