// The pure parts of HealthGrid: what order cells appear in and how a day of history reads aloud.

import type { PillState } from "./StatusPill.svelte";
import { t } from "../i18n/index.ts";

/** One bar of the 24-hour strip, one per hour: fine, degraded, failing, or no data. */
export type HealthBar = "ok" | "warn" | "err" | "none";

export type HealthItem = {
  id: string;
  name: string;
  /** A service mark for `BrandLogo`; when absent, `icon` is drawn instead. */
  brand?: string;
  icon?: string;
  state: PillState;
  /** The last 24 hours, oldest first, one bar an hour. Shorter histories are padded with `none` on the left. */
  history: HealthBar[];
  /** The one figure that matters, for example "p95 2.1 s" or "403 since 08:14". */
  note: string;
};

const HOURS = 24;

// Failing first, then what needs a person, then what is moving, then what is quiet, then what is fine.
const RANK: Record<PillState, number> = { failed: 0, needs_login: 1, restarting: 2, review: 3, starting: 4, stopped: 5, off: 6, ready: 7 };

/** The cells in the order the grid shows them: failing first, then by name. The input is not changed. */
export function sortFailingFirst(items: readonly HealthItem[]): HealthItem[] {
  return [...items].sort((a, b) => RANK[a.state] - RANK[b.state] || a.name.localeCompare(b.name));
}

/** Exactly 24 bars: older hours missing from `history` become `none`, extra old hours are dropped. */
export function lastDay(history: readonly HealthBar[]): HealthBar[] {
  const recent = history.slice(-HOURS);
  return [...Array<HealthBar>(HOURS - recent.length).fill("none"), ...recent];
}

/** The strip in words, for a screen reader: "Last 24 hours: 22 fine, 1 degraded, 1 failing." */
export function describeDay(history: readonly HealthBar[]): string {
  const bars = lastDay(history);
  const count = (kind: HealthBar) => bars.filter((b) => b === kind).length;
  const parts = [`${count("ok")} fine`];
  if (count("warn")) parts.push(`${count("warn")} degraded`);
  if (count("err")) parts.push(`${count("err")} failing`);
  if (count("none")) parts.push(t("ui.health.withoutData", { value: count("none") }));
  return t("ui.health.last24Hours", { value: parts.join(", ") });
}
