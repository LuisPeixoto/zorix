import assert from "node:assert/strict";
import { test } from "node:test";

import type { ActivityLine, Worker, WorkerView } from "./api.ts";
import {
  CARD_H,
  CARD_W,
  CHIP_H,
  CHIP_W,
  budgetShare,
  cardFigures,
  duration,
  evalText,
  filterLines,
  fitScale,
  layoutChart,
  levelsUnder,
  limitsSentence,
  lineTime,
  money,
  onTimeText,
  spendDelta,
  stillAskName,
} from "./workforce.ts";

const worker = (extra: Partial<Worker> = {}): Worker => ({
  tenant: "local",
  agent: "forge",
  role: "Engineering lead",
  department: "engineering",
  reports_to: null,
  manager: false,
  autonomy: "budgeted",
  weekly_budget_micros: 25_000_000,
  task_time_ms: 2 * 3_600_000,
  still_ask: [],
  schedule: { enabled: false, plan: { type: "every", minutes: 60 }, report: { type: "every", minutes: 60 } },
  revision: 1,
  hired_at: 0,
  updated_at: 0,
  ...extra,
});

const figures = (extra = {}) => ({ tasks: 6, on_time_done: 11, on_time_all: 12, eval: null, week_micros: 19_000_000, waiting: 0, ...extra });

const view = (w: Partial<Worker>, f = {}, own = {}, reports: string[] = []): WorkerView => ({
  worker: worker(w),
  name: "Forge",
  description: "",
  avatar: { emoji: "🛠", color: "warn" },
  status: null,
  figures: figures(f),
  own: figures(own),
  reports,
  budget_left_micros: null,
  archived: false,
});

test("durations read in hours and minutes", () => {
  assert.equal(duration(2 * 3_600_000), "2 h");
  assert.equal(duration(45 * 60_000), "45 min");
  assert.equal(duration(90 * 60_000), "1 h 30 min");
});

test("money is whole dollars when it is whole and cents otherwise", () => {
  assert.equal(money(25_000_000), "$25");
  assert.equal(money(12_500_000), "$12.50");
  assert.equal(money(0), "$0");
});

test("a repository the worker always asks about is named without its family and its write mark", () => {
  assert.equal(stillAskName("repo:acme/web:write"), "acme/web");
  assert.equal(stillAskName("db:analytics"), "analytics");
  assert.equal(stillAskName("read:docs/*"), "read:docs/*");
});

test("the sentence under the dial says what the level lets it do, within what, and what still asks", () => {
  assert.equal(limitsSentence(worker({ still_ask: ["repo:acme/web:write"] })), "Acts within $25 a week and 2 h a task. Writes to acme/web still ask.");
  assert.equal(limitsSentence(worker({ autonomy: "suggest" })), "Proposes; you act. Nothing it wants to change runs.");
  assert.equal(limitsSentence(worker({ autonomy: "notify", weekly_budget_micros: null })), "Acts, then tells you, within 2 h a task.");
  assert.equal(limitsSentence(worker({ still_ask: ["repo:a/b:write", "db:ops", "repo:c/d:write"] })), "Acts within $25 a week and 2 h a task. Writes to a/b, ops and c/d still ask.");
});

test("a worker may have any level under you and none above its manager", () => {
  assert.equal(levelsUnder(null).length, 5);
  assert.deepEqual(levelsUnder("notify"), ["suggest", "ask", "notify"]);
  assert.deepEqual(levelsUnder("suggest"), ["suggest"]);
});

test("the figures on a card follow the role: a manager shows its on-time rate, a worker that waits shows how many, else the eval", () => {
  const manager = cardFigures(view({ manager: true }, { tasks: 14 }, {}, ["iris", "forge"]));
  assert.deepEqual(manager.map((f) => f[1]), ["tasks", "on time", "week"]);
  assert.equal(manager[1][0], "92%");
  const waiting = cardFigures(view({}, { waiting: 2 }, { waiting: 2 }));
  assert.deepEqual(waiting[1], ["2", "waiting"]);
  const scored = cardFigures(view({}, {}, { eval: 0.94 }));
  assert.deepEqual(scored[1], ["0.94", "eval"]);
  const none = cardFigures(view({}, { on_time_done: 0, on_time_all: 0 }));
  assert.deepEqual(none[1], ["—", "on time"]);
  assert.equal(cardFigures(view({}, { tasks: 1 }))[0][1], "task");
});

test("on time and eval read as a dash when there is nothing behind them", () => {
  assert.equal(onTimeText(0, 0), "—");
  assert.equal(onTimeText(1, 3), "33%");
  assert.equal(evalText(null), "—");
  assert.equal(evalText(0.9), "0.90");
});

test("more spend is a warning and less is good, whatever the direction", () => {
  assert.deepEqual(spendDelta(118, 100), { text: "▲ 18%", tone: "warn" });
  assert.deepEqual(spendDelta(80, 100), { text: "▼ 20%", tone: "ok" });
  assert.equal(spendDelta(10, 0), null);
  assert.equal(spendDelta(100, 100), null);
});

test("the budget share is none without a budget and never above one", () => {
  assert.equal(budgetShare({ week_micros: 19_000_000 }, 25_000_000), 0.76);
  assert.equal(budgetShare({ week_micros: 30_000_000 }, 25_000_000), 1);
  assert.equal(budgetShare({ week_micros: 1 }, null), null);
});

const tree = [
  { id: "you", parent: null },
  { id: "atlas", parent: "you" },
  { id: "writer", parent: "you" },
  { id: "iris", parent: "atlas" },
  { id: "forge", parent: "atlas" },
  { id: "scout", parent: "iris" },
  { id: "analyst", parent: "iris" },
  { id: "reviewer", parent: "forge" },
];

test("the chart makes cards for three levels and chips below, and every parent sits over its children", () => {
  const chart = layoutChart(tree);
  const at = (id: string) => chart.nodes.find((n) => n.id === id)!;
  assert.equal(chart.nodes.length, tree.length);
  assert.deepEqual([at("you").chip, at("atlas").chip, at("iris").chip, at("scout").chip], [false, false, false, true]);
  assert.equal(at("scout").w, CHIP_W);
  assert.equal(at("scout").h, CHIP_H);
  assert.equal(at("atlas").w, CARD_W);
  assert.equal(at("atlas").h, CARD_H);
  const centre = (id: string) => at(id).x + at(id).w / 2;
  assert.ok(Math.abs(centre("iris") - (centre("scout") + centre("analyst")) / 2) < 0.001, "a parent is centred over its children");
  assert.ok(at("atlas").y > at("you").y && at("iris").y > at("atlas").y && at("scout").y > at("iris").y, "each level sits below the one above");
});

test("no two boxes of the chart overlap and all of them fit its size", () => {
  const chart = layoutChart(tree);
  for (const a of chart.nodes) {
    assert.ok(a.x >= -0.001 && a.x + a.w <= chart.width + 0.001, `${a.id} is inside the width`);
    assert.ok(a.y + a.h <= chart.height + 0.001, `${a.id} is inside the height`);
    for (const b of chart.nodes) {
      if (a.id >= b.id) continue;
      const apart = a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
      assert.ok(apart, `${a.id} and ${b.id} overlap`);
    }
  }
});

test("there is one connector for each parent that has children, and an empty chart is empty", () => {
  assert.equal(layoutChart(tree).paths.length, 4);
  const empty = layoutChart([]);
  assert.deepEqual([empty.nodes.length, empty.paths.length, empty.width, empty.height], [0, 0, 0, 0]);
  const alone = layoutChart([{ id: "you", parent: null }]);
  assert.equal(alone.width, CARD_W);
  assert.equal(alone.paths.length, 0);
});

test("a chart is scaled down to the room it has and never up", () => {
  assert.equal(fitScale(900, 450), 0.5);
  assert.equal(fitScale(400, 800), 1);
  assert.equal(fitScale(0, 800), 1);
});

const line = (level: ActivityLine["level"], at = 0): ActivityLine => ({ id: String(Math.random()), at, level, source: "forge", text: "x", kind: "org", run: null });

test("warn and above keeps waiting, warnings and errors", () => {
  const lines = [line("OK"), line("INFO"), line("WAIT"), line("WARN"), line("ERR")];
  assert.deepEqual(filterLines(lines, "warn").map((l) => l.level), ["WAIT", "WARN", "ERR"]);
  assert.equal(filterLines(lines, "all").length, 5);
});

test("a line's time is the clock today and the weekday before", () => {
  const now = new Date(2026, 10, 18, 12, 0, 0).getTime();
  const today = new Date(2026, 10, 18, 9, 5, 7).getTime();
  const earlier = new Date(2026, 10, 16, 9, 5, 7).getTime();
  assert.equal(lineTime(today, now), "09:05:07");
  assert.equal(lineTime(earlier, now), "Mon 09:05");
});

test("the chart's nodes put you at the top and each worker under who it reports to", async () => {
  const { orgNodes } = await import("./workforce.ts");
  const nodes = orgNodes({
    you: { to_review: 3, approvals: 2, reports: ["atlas"] },
    workers: [view({ agent: "atlas", manager: true, reports_to: null }, {}, {}, ["forge"]), view({ agent: "forge", reports_to: "atlas" })],
  });
  assert.deepEqual(nodes.map((n) => [n.id, n.parent]), [["you", null], ["atlas", "you"], ["forge", "atlas"]]);
  assert.equal(nodes[0].human, true);
  assert.deepEqual(nodes[0].figures[0], ["3", "to review"]);
  assert.ok(nodes[1].role.startsWith("Virtual manager"));
  assert.ok(nodes[2].label.includes("Budgeted"));
});
