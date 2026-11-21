// The numbers the shell shows around the app: today's spend, what is running, and the Runs page filters. Pure, tested in shell.test.ts.
import type { Execution, ExecutionState } from "./api.ts";
import { isFinished } from "./format.ts";

/** Money as the design system writes it: `$9.80`, and four digits while it is still under a cent. */
export function usd(micros: number): string {
  const dollars = micros / 1_000_000;
  if (dollars === 0) return "$0.00";
  return dollars < 0.01 ? `$${dollars.toFixed(4)}` : `$${dollars.toFixed(2)}`;
}

/** Midnight, local time, of the day `now` falls on. */
export function startOfDay(now: number): number {
  const day = new Date(now);
  day.setHours(0, 0, 0, 0);
  return day.getTime();
}

/** What the runs started today cost, in micro-dollars. Sub-runs are already counted in the run that started them. */
export function spendToday(runs: readonly Pick<Execution, "created_at" | "usage">[], now: number): number {
  const from = startOfDay(now);
  return runs.filter((r) => r.created_at >= from).reduce((sum, r) => sum + r.usage.cost_micros, 0);
}

const WORKING: ExecutionState[] = ["queued", "running", "waiting_for_model", "waiting_for_tool", "waiting_for_children"];

/** Runs doing work right now (not paused, not waiting for a person, not finished). */
export const runningCount = (runs: readonly Pick<Execution, "state">[]): number => runs.filter((r) => WORKING.includes(r.state)).length;

export type RunFilter = "all" | "running" | "waiting" | "finished";

/** True when a run belongs under a filter of the Runs page. */
export function inFilter(run: Pick<Execution, "state">, filter: RunFilter): boolean {
  if (filter === "all") return true;
  if (filter === "running") return WORKING.includes(run.state);
  if (filter === "waiting") return run.state === "waiting_for_approval" || run.state === "paused";
  return isFinished(run.state);
}
