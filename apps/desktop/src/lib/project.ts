// Projects and tasks: the pure rules the screens use. Dates are days with no time zone ("2026-12-20"); the rules that decide
// anything (moves, runs, approvals) live in Rust. Tested in project.test.ts.
import type { Counts, NeedsYou, ProjectEvent, TaskPriority, TaskState, TaskView, Who } from "./api.ts";
import type { UiTone } from "./ui/tone.ts";
import { fmt, t } from "./i18n/index.ts";

// ---------------------------------------------------------------------------------------------------------------------
// Lanes, priorities, words

export interface Lane {
  id: TaskState;
  name: string;
  tone: UiTone;
}

export const LANES: Lane[] = [
  { id: "backlog", name: t("project.backlog"), tone: "muted" },
  { id: "in_progress", name: t("project.inProgress"), tone: "info" },
  { id: "review", name: t("common.review"), tone: "warn" },
  { id: "done", name: t("common.done"), tone: "ok" },
];

export const LANE_NAME: Record<TaskState, string> = { backlog: t("project.backlog"), in_progress: t("project.inProgress"), review: t("common.review"), done: t("common.done") };
export const LANE_TONE: Record<TaskState, UiTone> = { backlog: "muted", in_progress: "info", review: "warn", done: "ok" };

// Written out in full so Tailwind can see every class.
export const PRIORITY: Record<TaskPriority, { label: string; text: string; order: number }> = {
  urgent: { label: t("project.urgent"), text: "text-priority-urgent", order: 0 },
  high: { label: t("project.high"), text: "text-priority-high", order: 1 },
  normal: { label: t("project.normal"), text: "text-priority-normal", order: 2 },
  low: { label: t("project.low"), text: "text-priority-low", order: 3 },
};

export const PRIORITIES: TaskPriority[] = ["urgent", "high", "normal", "low"];

export const NEEDS_TEXT: Record<NeedsYou, { label: string; tone: UiTone }> = {
  review: { label: t("common.needsYou"), tone: "warn" },
  run_failed: { label: t("project.runFailed"), tone: "err" },
  run_waiting: { label: t("common.waitingForYou"), tone: "warn" },
};

export const keyNumber = (key: string): number => Number(key.slice(key.lastIndexOf("-") + 1)) || 0;

// ---------------------------------------------------------------------------------------------------------------------
// Days

const DAY_MS = 86_400_000;

/** Days since 1970-01-01 of a `YYYY-MM-DD` day. */
export function dayNumber(day: string): number {
  const [y, m, d] = day.split("-").map(Number);
  return Math.round(Date.UTC(y ?? 1970, (m ?? 1) - 1, d ?? 1) / DAY_MS);
}

export function dayText(n: number): string {
  const date = new Date(n * DAY_MS);
  const pad = (v: number) => String(v).padStart(2, "0");
  return `${date.getUTCFullYear()}-${pad(date.getUTCMonth() + 1)}-${pad(date.getUTCDate())}`;
}

/** Today as a day, in the person's own time zone. */
export function todayOf(now: number): string {
  const date = new Date(now);
  const pad = (v: number) => String(v).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

export function addDays(day: string, n: number): string {
  return dayText(dayNumber(day) + n);
}

/** "Dec 12". */
export function dayLabel(day: string): string {
  return fmt.day(new Date(dayNumber(day) * DAY_MS), { month: "short", day: "numeric", timeZone: "UTC" });
}

/** What a card shows for a due date: a late one is `err` and says how late; any other is the day. */
export function dueLabel(due: string | null, today: string): { text: string; late: boolean } | null {
  if (!due) return null;
  const late = dayNumber(today) - dayNumber(due);
  if (late === 0) return { text: t("project.today"), late: true };
  if (late > 0) return { text: late === 1 ? t("project.1DayLate") : t("project.daysLate", { late }), late: true };
  return { text: dayLabel(due), late: false };
}

// ---------------------------------------------------------------------------------------------------------------------
// Board

export type Filter = "everyone" | "agents" | "people" | "urgent";
export const FILTERS: { id: Filter; label: string }[] = [
  { id: "everyone", label: t("project.everyone") },
  { id: "agents", label: t("common.agents") },
  { id: "people", label: t("project.people") },
  { id: "urgent", label: t("project.urgentAndHigh") },
];

export function filterTasks(views: TaskView[], filter: Filter): TaskView[] {
  switch (filter) {
    case "agents":
      return views.filter((v) => v.task.assignee && v.task.assignee.type !== "you");
    case "people":
      return views.filter((v) => v.task.assignee?.type === "you");
    case "urgent":
      return views.filter((v) => v.task.priority === "urgent" || v.task.priority === "high");
    default:
      return views;
  }
}

export function byLane(views: TaskView[]): Record<TaskState, TaskView[]> {
  const out: Record<TaskState, TaskView[]> = { backlog: [], in_progress: [], review: [], done: [] };
  for (const view of views) out[view.task.state].push(view);
  for (const lane of LANES) out[lane.id].sort((a, b) => a.task.rank - b.task.rank || keyNumber(a.task.key) - keyNumber(b.task.key));
  return out;
}

export const countOf = (counts: Counts): number => counts.backlog + counts.in_progress + counts.review + counts.done;

/** "18 of 29 tasks" and how full the bar is. */
export function progressOf(counts: Counts): { text: string; percent: number } {
  const total = countOf(counts);
  return { text: `${counts.done} of ${total} ${total === 1 ? "task" : "tasks"}`, percent: total === 0 ? 0 : Math.round((counts.done / total) * 100) };
}

export function countsOf(views: TaskView[]): Counts {
  const counts: Counts = { backlog: 0, in_progress: 0, review: 0, done: 0 };
  for (const view of views) counts[view.task.state] += 1;
  return counts;
}

export function whoName(who: Who | null, agentName: (slug: string) => string): string {
  if (!who) return t("common.nobody");
  return who.type === "you" ? t("common.you") : agentName(who.slug);
}

export function whoKey(who: Who | null): string {
  return who ? (who.type === "you" ? "you" : `${who.type}:${who.slug}`) : "none";
}

/** Reads `you`, `agent:SLUG`, `team:SLUG` or `none` (what a select holds). */
export function whoOf(key: string): Who | null {
  if (key === "you") return { type: "you" };
  if (key.startsWith("agent:")) return { type: "agent", slug: key.slice(6) };
  if (key.startsWith("team:")) return { type: "team", slug: key.slice(5) };
  return null;
}

export interface CardStatus {
  text: string;
  tone: UiTone;
  live: boolean;
}

/** The one thing a card says about itself besides its title: what needs you, what blocks it, who is at work on it. */
export function cardStatus(view: TaskView, manager: string | null, agentName: (slug: string) => string): CardStatus | null {
  if (view.needs_you) return { text: NEEDS_TEXT[view.needs_you].label, tone: NEEDS_TEXT[view.needs_you].tone, live: false };
  if (view.blocked_by.length > 0 && view.task.state === "in_progress") {
    return { text: t("project.waitingFor", { value: view.blocked_by[0], value2: view.blocked_by.length > 1 ? ` and ${view.blocked_by.length - 1} more` : "" }), tone: "muted", live: false };
  }
  const who = view.task.assignee;
  if (view.working && who && who.type !== "you") {
    const name = agentName(who.slug);
    return { text: who.type === "agent" && who.slug === manager ? t("project.isPlanning", { name }) : t("project.isWorking", { name }), tone: "info", live: true };
  }
  return null;
}

/**
 * Where a card goes when a key moves it. `order` lists the lanes left to right and `others` how many other cards each has (without
 * the one being moved), so a place is 0 up to that many. Left and right keep the place as far as the next lane allows; up and down
 * move within the lane.
 */
export function nextSpot(
  order: string[],
  others: Record<string, number>,
  at: { lane: string; index: number },
  key: string,
): { lane: string; index: number } {
  const lane = order.indexOf(at.lane);
  if (key === "ArrowLeft" || key === "ArrowRight") {
    const next = order[Math.min(order.length - 1, Math.max(0, lane + (key === "ArrowRight" ? 1 : -1)))] ?? at.lane;
    if (next === at.lane) return at;
    return { lane: next, index: Math.min(at.index, others[next] ?? 0) };
  }
  if (key === "ArrowUp" || key === "ArrowDown") {
    return { lane: at.lane, index: Math.min(others[at.lane] ?? 0, Math.max(0, at.index + (key === "ArrowDown" ? 1 : -1))) };
  }
  return at;
}

/** The place in a lane for a pointer at height `y`, given the middle heights of the cards in it (without the dragged card). */
export function dropIndex(middles: number[], y: number): number {
  let index = 0;
  for (const middle of middles) {
    if (y > middle) index += 1;
  }
  return index;
}

// ---------------------------------------------------------------------------------------------------------------------
// Links

/** What a link says about itself: a pull request shows its number. */
export function linkLabel(url: string, title: string | null): { kind: "pr" | "link"; text: string } {
  const github = /^https:\/\/github\.com\/[^/]+\/[^/]+\/pull\/(\d+)/.exec(url);
  if (github) return { kind: "pr", text: t("project.pr", { value: github[1] }) };
  const gitlab = /^https?:\/\/[^/]*gitlab[^/]*\/.+\/-\/merge_requests\/(\d+)/.exec(url);
  if (gitlab) return { kind: "pr", text: t("project.mr", { value: gitlab[1] }) };
  if (title) return { kind: "link", text: title };
  try {
    const parsed = new URL(url);
    return { kind: "link", text: parsed.host + (parsed.pathname === "/" ? "" : parsed.pathname) };
  } catch {
    return { kind: "link", text: url };
  }
}

// ---------------------------------------------------------------------------------------------------------------------
// Timeline

export type Scale = "days" | "weeks" | "months";

export interface Column {
  label: string;
  start: number;
  days: number;
}

export interface Bar {
  view: TaskView;
  row: number;
  start: number;
  end: number;
  /** Where it starts and how wide it is, as fractions of the whole range (clipped to it). */
  left: number;
  width: number;
  tone: UiTone;
}

export interface Timeline {
  columns: Column[];
  bars: Bar[];
  /** Where today is, as a fraction of the range, or null when it is outside. */
  today: number | null;
  /** Tasks with no date: they are listed under the grid. */
  unscheduled: TaskView[];
  from: number;
  total: number;
}

export function barTone(view: TaskView): UiTone {
  if (view.task.state === "done") return "ok";
  switch (view.task.priority) {
    case "urgent":
      return "err";
    case "high":
      return "warn";
    case "low":
      return "muted";
    default:
      return "accent";
  }
}

/** The Monday on or before a day number. */
function mondayOf(n: number): number {
  const weekday = (new Date(n * DAY_MS).getUTCDay() + 6) % 7;
  return n - weekday;
}

function monthStart(n: number): number {
  const date = new Date(n * DAY_MS);
  return Math.round(Date.UTC(date.getUTCFullYear(), date.getUTCMonth(), 1) / DAY_MS);
}

function nextMonth(n: number): number {
  const date = new Date(n * DAY_MS);
  return Math.round(Date.UTC(date.getUTCFullYear(), date.getUTCMonth() + 1, 1) / DAY_MS);
}

export function buildTimeline(views: TaskView[], scale: Scale, today: string): Timeline {
  const now = dayNumber(today);
  const scheduled = views.filter((v) => v.task.start || v.task.due);
  const unscheduled = views.filter((v) => !v.task.start && !v.task.due);
  const spans = scheduled.map((v) => {
    const start = dayNumber(v.task.start ?? (v.task.due as string));
    const end = Math.max(start, dayNumber(v.task.due ?? (v.task.start as string)));
    return { view: v, start, end };
  });
  spans.sort((a, b) => a.start - b.start || a.end - b.end || keyNumber(a.view.task.key) - keyNumber(b.view.task.key));
  const earliest = Math.min(now - 14, ...spans.map((s) => s.start));
  const latest = Math.max(now + 14, ...spans.map((s) => s.end));

  const columns: Column[] = [];
  if (scale === "days") {
    const first = Math.min(now - 3, spans.length ? Math.max(earliest, now - 10) : now - 3);
    const count = Math.min(60, Math.max(21, latest - first + 1));
    for (let i = 0; i < count; i += 1) {
      const date = new Date((first + i) * DAY_MS);
      columns.push({ label: fmt.day(date, { month: "short", day: "numeric", timeZone: "UTC" }), start: first + i, days: 1 });
    }
  } else if (scale === "weeks") {
    const first = mondayOf(earliest);
    const count = Math.min(26, Math.max(8, Math.ceil((latest - first + 1) / 7)));
    for (let i = 0; i < count; i += 1) {
      const date = new Date((first + i * 7) * DAY_MS);
      columns.push({ label: fmt.day(date, { month: "short", day: "numeric", timeZone: "UTC" }), start: first + i * 7, days: 7 });
    }
  } else {
    let cursor = monthStart(earliest);
    const count = Math.min(18, Math.max(6, Math.ceil((latest - cursor) / 30)));
    for (let i = 0; i < count; i += 1) {
      const next = nextMonth(cursor);
      const date = new Date(cursor * DAY_MS);
      columns.push({ label: fmt.day(date, { month: "short", year: "numeric", timeZone: "UTC" }), start: cursor, days: next - cursor });
      cursor = next;
    }
  }
  const from = columns[0]?.start ?? now;
  const total = columns.reduce((sum, c) => sum + c.days, 0) || 1;
  const bars: Bar[] = spans.map((s, row) => {
    const left = Math.max(0, (s.start - from) / total);
    const right = Math.min(1, (s.end + 1 - from) / total);
    return { view: s.view, row, start: s.start, end: s.end, left: Math.min(left, 1), width: Math.max(0, right - left), tone: barTone(s.view) };
  });
  const today_ = (now - from + 0.5) / total;
  return { columns, bars, today: today_ >= 0 && today_ <= 1 ? today_ : null, unscheduled, from, total };
}

export interface Arrow {
  from: string;
  to: string;
  path: string;
  head: string;
  /** The task starts before the one it waits for is due. */
  late: boolean;
}

/** The arrows from the end of a task to the start of the tasks that wait for it, in pixels of a lane `width` wide. */
export function dependencyArrows(bars: Bar[], width: number, rowHeight: number): Arrow[] {
  const byKey = new Map(bars.map((b) => [b.view.task.key, b]));
  const arrows: Arrow[] = [];
  const centre = (row: number) => row * rowHeight + rowHeight / 2;
  for (const bar of bars) {
    for (const dep of bar.view.task.depends_on) {
      const before = byKey.get(dep);
      if (!before) continue;
      const x1 = (before.left + before.width) * width;
      const y1 = centre(before.row);
      const x2 = bar.left * width;
      const y2 = centre(bar.row);
      const late = before.end >= bar.start;
      const elbow = late
        ? `M${x1} ${y1} H${x1 + 12} V${y2 - rowHeight / 2} H${x2 - 12} V${y2} H${x2 - 6}`
        : `M${x1} ${y1} H${Math.min(x1 + 12, (x1 + x2) / 2)} V${y2} H${x2 - 6}`;
      arrows.push({ from: dep, to: bar.view.task.key, path: elbow, head: `${x2},${y2} ${x2 - 7},${y2 - 3.5} ${x2 - 7},${y2 + 3.5}`, late });
    }
  }
  return arrows;
}

// ---------------------------------------------------------------------------------------------------------------------
// History

export type EventGroup = "moves" | "runs" | "approvals" | "other";

export function eventGroup(event: ProjectEvent): EventGroup {
  const k = event.kind;
  if (k.type === "moved" && k.approved_by) return "approvals";
  switch (k.type) {
    case "moved":
    case "task_created":
    case "assigned":
      return "moves";
    case "run_started":
    case "run_finished":
    case "run_failed":
    case "run_cancelled":
    case "start_failed":
    case "blocked":
    case "manager_asked":
      return "runs";
    default:
      return "other";
  }
}

/** One line of history in words. `name` turns an agent's slug into what people call it. */
export function eventSentence(event: ProjectEvent, name: (slug: string) => string): string {
  const who = event.by ? (event.by.type === "you" ? t("common.you") : name(event.by.slug)) : "ZORIX";
  const task = event.task ?? t("project.theProject");
  const k = event.kind;
  switch (k.type) {
    case "project_created":
      return t("project.madeTheProject", { who });
    case "project_edited":
      return t("project.changedTheProject", { who });
    case "project_archived":
      return t("project.theProject2", { who, value: k.archived ? "archived" : t("project.broughtBack") });
    case "task_created":
      return `${who} added ${task}: ${k.title}`;
    case "task_edited":
      return `${who} changed ${task}`;
    case "moved":
      return `${who} ${k.approved_by ? t("project.askedToMove") : "moved"} ${task} ${k.approved_by ? "to" : `from ${LANE_NAME[k.from]} to`} ${LANE_NAME[k.to]}${k.note ? `: “${k.note}”` : ""}`;
    case "assigned":
      return k.to ? `${who} gave ${task} to ${k.to.type === "you" ? "you" : name(k.to.slug)}` : t("project.tookOffWhoeverHad", { who, task });
    case "run_started":
      return t("project.aRunStartedOn", { task });
    case "run_finished":
      return t("project.theRunOnFinished", { task });
    case "run_failed":
      return t("project.theRunOnFailed", { task, reason: k.reason });
    case "start_failed":
      return t("project.aRunCouldNot", { task, reason: k.reason });
    case "run_cancelled":
      return t("project.theRunOnWas", { task });
    case "blocked":
      return t("project.waitsFor", { task, value: k.by.join(", ") });
    case "progress":
      return t("project.reportedOn", { who, percent: k.percent, task });
    case "deliverable":
      return t("project.addedADeliverableTo", { who, task, title: k.title });
    case "comment":
      return t("project.commentedOn", { who, task, text: k.text });
    case "linked":
      return `${who} linked ${k.url} to ${task}`;
    case "manager_asked":
      return t("project.askedTheManagerTo", { who });
  }
}

/** Events in groups by day, newest first, with "Today" and "Yesterday" named. */
export function groupByDay(events: ProjectEvent[], today: string): { label: string; events: ProjectEvent[] }[] {
  const out: { label: string; day: string; events: ProjectEvent[] }[] = [];
  for (const event of events) {
    const day = todayOf(event.at);
    const last = out[out.length - 1];
    if (last && last.day === day) last.events.push(event);
    else out.push({ label: day === today ? t("project.today") : day === addDays(today, -1) ? t("project.yesterday") : dayLabel(day), day, events: [event] });
  }
  return out.map(({ label, events: list }) => ({ label, events: list }));
}

/** The time of day of an event, `09:31`. */
export function clock(at: number): string {
  return fmt.clock(at);
}

/** A cost in dollars from millionths. */
export const usd = (micros: number): string => fmt.usd2(micros);

// ---------------------------------------------------------------------------------------------------------------------
// Forms and sorting

/** A key to start from, taken from a name: "Q4 Operations" gives QO, "Website" gives WEB. */
export function suggestKey(name: string): string {
  const words = name
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .toUpperCase()
    .split(/[^A-Z0-9]+/)
    .filter(Boolean);
  if (words.length === 0) return "";
  const letters = words.length === 1 ? (words[0] as string).slice(0, 3) : words.map((w) => w[0]).join("").slice(0, 4);
  return /^[A-Z]/.test(letters) ? letters : `P${letters}`.slice(0, 5);
}

export const isProjectKey = (key: string): boolean => /^[A-Z][A-Z0-9]{1,4}$/.test(key);

/** Dollars typed by a person to millionths, or null for nothing; NaN-safe: a bad number gives undefined. */
export function usdToMicros(text: string): number | null | undefined {
  const clean = text.trim().replace(/^\$/, "");
  if (clean === "") return null;
  const value = Number(clean);
  if (!Number.isFinite(value) || value <= 0 || value > 100_000) return undefined;
  return Math.round(value * 1_000_000);
}

export const microsToUsd = (micros: number | null): string => (micros === null ? "" : String(micros / 1_000_000));

export type SortBy = "key" | "priority" | "state" | "progress" | "due";

/** The list, ordered by a column. Tasks with no due date go last whichever way the dates run. */
export function sortTasks(views: TaskView[], by: SortBy, ascending: boolean): TaskView[] {
  const lane = LANES.map((l) => l.id);
  const value = (v: TaskView): number | string => {
    switch (by) {
      case "key":
        return keyNumber(v.task.key);
      case "priority":
        return PRIORITY[v.task.priority].order;
      case "state":
        return lane.indexOf(v.task.state);
      case "progress":
        return v.task.progress;
      case "due":
        return v.task.due ?? "";
    }
  };
  const sign = ascending ? 1 : -1;
  return [...views].sort((a, b) => {
    if (by === "due") {
      if (!a.task.due || !b.task.due) return a.task.due ? -1 : b.task.due ? 1 : keyNumber(a.task.key) - keyNumber(b.task.key);
    }
    const x = value(a);
    const y = value(b);
    const order = x < y ? -1 : x > y ? 1 : 0;
    return order * sign || keyNumber(a.task.key) - keyNumber(b.task.key);
  });
}
