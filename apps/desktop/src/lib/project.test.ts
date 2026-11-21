import assert from "node:assert/strict";
import { test } from "node:test";

import type { ProjectEvent, Task, TaskView } from "./api.ts";
import {
  addDays,
  buildTimeline,
  byLane,
  cardStatus,
  countsOf,
  dayLabel,
  dayNumber,
  dependencyArrows,
  dropIndex,
  dueLabel,
  eventGroup,
  eventSentence,
  filterTasks,
  groupByDay,
  linkLabel,
  nextSpot,
  isProjectKey,
  microsToUsd,
  progressOf,
  sortTasks,
  suggestKey,
  todayOf,
  usdToMicros,
  whoKey,
  whoOf,
} from "./project.ts";

function task(key: string, over: Partial<Task> = {}): Task {
  return {
    id: key,
    project: "p",
    key,
    title: `Task ${key}`,
    description: "",
    state: "backlog",
    priority: "normal",
    assignee: null,
    progress: 0,
    start: null,
    due: null,
    depends_on: [],
    links: [],
    deliverables: [],
    runs: [],
    run_requested: null,
    rank: 0,
    revision: 0,
    created_at: 0,
    updated_at: 0,
    ...over,
  };
}

function view(key: string, over: Partial<Task> = {}, rest: Partial<TaskView> = {}): TaskView {
  return { task: task(key, over), blocked_by: [], needs_you: null, working: false, ...rest };
}

const name = (slug: string) => slug.charAt(0).toUpperCase() + slug.slice(1);

test("a due date is late in words, and any other is the day", () => {
  assert.deepEqual(dueLabel("2026-11-21", "2026-11-21"), { text: "Today", late: true });
  assert.deepEqual(dueLabel("2026-11-19", "2026-11-21"), { text: "2 days late", late: true });
  assert.deepEqual(dueLabel("2026-11-20", "2026-11-21"), { text: "1 day late", late: true });
  assert.deepEqual(dueLabel("2026-12-12", "2026-11-21"), { text: "Dec 12", late: false });
  assert.equal(dueLabel(null, "2026-11-21"), null);
  assert.equal(dayLabel("2026-02-03"), "Feb 3");
  assert.equal(addDays("2026-12-30", 3), "2027-01-02");
  assert.equal(dayNumber("2026-03-01") - dayNumber("2026-02-28"), 1);
  assert.equal(todayOf(new Date(2026, 10, 21, 23, 30).getTime()), "2026-11-21");
});

test("the board groups tasks by lane in rank order and the filters pick who has them", () => {
  const views = [
    view("OPS-3", { state: "backlog", rank: 2000 }),
    view("OPS-1", { state: "backlog", rank: 1000, assignee: { type: "you" } }),
    view("OPS-2", { state: "in_progress", priority: "urgent", assignee: { type: "agent", slug: "scout" } }),
    view("OPS-4", { state: "done", priority: "high", assignee: { type: "team", slug: "desk" } }),
  ];
  const lanes = byLane(views);
  assert.deepEqual(lanes.backlog.map((v) => v.task.key), ["OPS-1", "OPS-3"]);
  assert.equal(lanes.in_progress.length + lanes.done.length, 2);
  assert.deepEqual(filterTasks(views, "agents").map((v) => v.task.key), ["OPS-2", "OPS-4"]);
  assert.deepEqual(filterTasks(views, "people").map((v) => v.task.key), ["OPS-1"]);
  assert.deepEqual(filterTasks(views, "urgent").map((v) => v.task.key), ["OPS-2", "OPS-4"]);
  assert.equal(filterTasks(views, "everyone").length, 4);
  assert.deepEqual(countsOf(views), { backlog: 2, in_progress: 1, review: 0, done: 1 });
  assert.deepEqual(progressOf({ backlog: 8, in_progress: 3, review: 2, done: 14 }), { text: "14 of 27 tasks", percent: 52 });
  assert.deepEqual(progressOf({ backlog: 0, in_progress: 0, review: 0, done: 0 }), { text: "0 of 0 tasks", percent: 0 });
});

test("a card says what needs you first, then what blocks it, then who is at work", () => {
  const manager = "atlas";
  assert.equal(cardStatus(view("OPS-1"), manager, name), null);
  assert.deepEqual(cardStatus(view("OPS-1", {}, { needs_you: "review" }), manager, name), { text: "Needs you", tone: "warn", live: false });
  assert.equal(cardStatus(view("OPS-1", {}, { needs_you: "run_failed" }), manager, name)?.tone, "err");
  const blocked = view("OPS-2", { state: "in_progress" }, { blocked_by: ["OPS-1", "OPS-3"] });
  assert.equal(cardStatus(blocked, manager, name)?.text, "Waiting for OPS-1 and 1 more");
  const working = view("OPS-3", { state: "in_progress", assignee: { type: "agent", slug: "scout" } }, { working: true });
  assert.deepEqual(cardStatus(working, manager, name), { text: "Scout is working", tone: "info", live: true });
  const planning = view("OPS-4", { state: "in_progress", assignee: { type: "agent", slug: "atlas" } }, { working: true });
  assert.equal(cardStatus(planning, manager, name)?.text, "Atlas is planning");
});

test("the keyboard moves a card across lanes and up and down a lane", () => {
  // Other cards in each lane, not counting the one that is moving: a place is 0 up to that many.
  const order = ["backlog", "in_progress", "review", "done"];
  const others = { backlog: 3, in_progress: 1, review: 0, done: 5 };
  const spot = (lane: string, index: number) => ({ lane, index });
  assert.deepEqual(nextSpot(order, others, spot("backlog", 2), "ArrowRight"), spot("in_progress", 1));
  assert.deepEqual(nextSpot(order, others, spot("in_progress", 0), "ArrowRight"), spot("review", 0));
  assert.deepEqual(nextSpot(order, others, spot("backlog", 0), "ArrowLeft"), spot("backlog", 0), "the first lane has nothing to its left");
  assert.deepEqual(nextSpot(order, others, spot("backlog", 1), "ArrowDown"), spot("backlog", 2));
  assert.deepEqual(nextSpot(order, others, spot("backlog", 3), "ArrowDown"), spot("backlog", 3), "the end of the lane is the last place");
  assert.deepEqual(nextSpot(order, others, spot("review", 0), "ArrowDown"), spot("review", 0), "an empty lane has one place");
  assert.deepEqual(nextSpot(order, others, spot("backlog", 0), "ArrowUp"), spot("backlog", 0));
  assert.deepEqual(nextSpot(order, others, spot("done", 4), "Enter"), spot("done", 4));
});

test("a drop lands after the cards whose middle it is below", () => {
  assert.equal(dropIndex([], 50), 0);
  assert.equal(dropIndex([40, 120, 200], 10), 0);
  assert.equal(dropIndex([40, 120, 200], 130), 2);
  assert.equal(dropIndex([40, 120, 200], 400), 3);
});

test("a link says what it is: a pull request shows its number", () => {
  assert.deepEqual(linkLabel("https://github.com/acme/api/pull/482", null), { kind: "pr", text: "PR #482" });
  assert.deepEqual(linkLabel("https://gitlab.com/g/p/-/merge_requests/7", null), { kind: "pr", text: "MR !7" });
  assert.deepEqual(linkLabel("https://example.com/runbook", "Runbook"), { kind: "link", text: "Runbook" });
  assert.deepEqual(linkLabel("https://example.com/runbook", null), { kind: "link", text: "example.com/runbook" });
});

test("who round-trips through the key a select holds", () => {
  assert.equal(whoKey({ type: "agent", slug: "scout" }), "agent:scout");
  assert.deepEqual(whoOf("team:desk"), { type: "team", slug: "desk" });
  assert.deepEqual(whoOf("you"), { type: "you" });
  assert.equal(whoOf("none"), null);
  assert.equal(whoKey(null), "none");
});

test("the timeline puts bars by day, today by fraction, and lists tasks with no dates apart", () => {
  const views = [
    view("OPS-1", { start: "2026-11-03", due: "2026-11-17", state: "done" }),
    view("OPS-2", { start: "2026-11-17", due: "2026-11-28", priority: "urgent" }),
    view("OPS-3", { due: "2026-12-02", priority: "high" }),
    view("OPS-4"),
  ];
  const t = buildTimeline(views, "weeks", "2026-11-21");
  assert.equal(t.columns.length, 8);
  assert.equal(t.columns[0]?.label, "Nov 2", "weeks start on a Monday (Nov 2 2026), at the first task or two weeks before today");
  assert.deepEqual(t.bars.map((b) => b.view.task.key), ["OPS-1", "OPS-2", "OPS-3"], "by start, undated ones left out");
  assert.deepEqual(t.unscheduled.map((v) => v.task.key), ["OPS-4"]);
  assert.deepEqual(t.bars.map((b) => b.tone), ["ok", "err", "warn"]);
  const first = t.bars[0];
  assert.ok(first && Math.abs(first.left - 1 / 56) < 1e-9 && Math.abs(first.width - 15 / 56) < 1e-9);
  assert.ok(t.today !== null && Math.abs(t.today - 19.5 / 56) < 1e-9);
  const days = buildTimeline(views, "days", "2026-11-21");
  assert.ok(days.columns.length >= 21 && days.columns.every((c) => c.days === 1));
  const months = buildTimeline(views, "months", "2026-11-21");
  assert.ok(months.columns.length >= 6 && months.columns[0]?.label === "Nov 2026");
  assert.equal(months.columns[0]?.days, 30);
  assert.equal(buildTimeline([], "weeks", "2026-11-21").bars.length, 0);
});

test("dependency arrows run from the end of a task to the start of the next, and are late when it starts too soon", () => {
  const views = [
    view("OPS-1", { start: "2026-11-03", due: "2026-11-10" }),
    view("OPS-2", { start: "2026-11-17", due: "2026-11-24", depends_on: ["OPS-1"] }),
    view("OPS-3", { start: "2026-11-05", due: "2026-11-20" }),
    view("OPS-4", { start: "2026-11-12", due: "2026-11-30", depends_on: ["OPS-3", "OPS-9"] }),
  ];
  const t = buildTimeline(views, "weeks", "2026-11-21");
  const arrows = dependencyArrows(t.bars, 800, 44);
  assert.equal(arrows.length, 2, "a dependency that is not drawn makes no arrow");
  const fine = arrows.find((a) => a.to === "OPS-2");
  const late = arrows.find((a) => a.to === "OPS-4");
  assert.equal(fine?.late, false);
  assert.equal(late?.late, true);
  assert.ok(fine?.path.startsWith("M") && late?.head.split(" ").length === 3);
});

test("history is told in sentences, grouped, and the filters know what an approval is", () => {
  const at = (day: string, h: number) => new Date(`${day}T${String(h).padStart(2, "0")}:00:00`).getTime();
  const event = (kind: ProjectEvent["kind"], by: ProjectEvent["by"], when: number, taskKey: string | null = "OPS-33"): ProjectEvent => ({
    seq: 1,
    project: "p",
    task: taskKey,
    by,
    kind,
    at: when,
  });
  const asked = event({ type: "moved", from: "review", to: "done", approved_by: "you" }, { type: "agent", slug: "atlas" }, at("2026-11-21", 9));
  assert.equal(eventSentence(asked, name), "Atlas asked to move OPS-33 to Done");
  assert.equal(eventGroup(asked), "approvals");
  const moved = event({ type: "moved", from: "backlog", to: "in_progress", note: "Add Friday." }, { type: "you" }, at("2026-11-20", 14));
  assert.equal(eventSentence(moved, name), "You moved OPS-33 from Backlog to In progress: “Add Friday.”");
  assert.equal(eventGroup(moved), "moves");
  const failed = event({ type: "run_failed", execution: "e", reason: "refused" }, null, at("2026-11-20", 15));
  assert.equal(eventSentence(failed, name), "The run on OPS-33 failed: refused");
  assert.equal(eventGroup(failed), "runs");
  assert.equal(eventGroup(event({ type: "comment", text: "hi" }, { type: "you" }, 0)), "other");
  assert.equal(eventSentence(event({ type: "project_created" }, null, 0, null), name), "ZORIX made the project");
  const groups = groupByDay([asked, moved, failed], "2026-11-21");
  assert.deepEqual(groups.map((g) => [g.label, g.events.length]), [["Today", 1], ["Yesterday", 2]]);
});

test("a key is suggested from a name and a cost is read from dollars", () => {
  assert.equal(suggestKey("Q4 Operations"), "QO");
  assert.equal(suggestKey("Website"), "WEB");
  assert.equal(suggestKey("Security hygiene and keys"), "SHAK");
  assert.equal(suggestKey("Café"), "CAF");
  assert.equal(suggestKey("2025 plan"), "P2P");
  assert.equal(suggestKey("   "), "");
  assert.ok(isProjectKey("OPS") && isProjectKey("WEB2") && !isProjectKey("ops") && !isProjectKey("A") && !isProjectKey("1AB") && !isProjectKey("TOOLONG"));
  assert.equal(usdToMicros(""), null);
  assert.equal(usdToMicros("$1.50"), 1_500_000);
  assert.equal(usdToMicros("0"), undefined);
  assert.equal(usdToMicros("abc"), undefined);
  assert.equal(microsToUsd(2_500_000), "2.5");
  assert.equal(microsToUsd(null), "");
});

test("the list sorts by a column and puts tasks with no due date last", () => {
  const views = [
    view("OPS-1", { due: "2026-12-02", priority: "low", state: "done", progress: 100 }),
    view("OPS-2", { priority: "urgent", state: "in_progress", progress: 40 }),
    view("OPS-3", { due: "2026-11-20", priority: "high", state: "backlog" }),
  ];
  const keys = (list: TaskView[]) => list.map((v) => v.task.key);
  assert.deepEqual(keys(sortTasks(views, "due", true)), ["OPS-3", "OPS-1", "OPS-2"]);
  assert.deepEqual(keys(sortTasks(views, "due", false)), ["OPS-1", "OPS-3", "OPS-2"], "no date stays last when descending");
  assert.deepEqual(keys(sortTasks(views, "priority", true)), ["OPS-2", "OPS-3", "OPS-1"]);
  assert.deepEqual(keys(sortTasks(views, "state", true)), ["OPS-3", "OPS-2", "OPS-1"]);
  assert.deepEqual(keys(sortTasks(views, "progress", false)), ["OPS-1", "OPS-2", "OPS-3"]);
  assert.deepEqual(keys(sortTasks(views, "key", false)), ["OPS-3", "OPS-2", "OPS-1"]);
});
