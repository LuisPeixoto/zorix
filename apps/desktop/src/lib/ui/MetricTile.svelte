<script lang="ts">
  import { t } from "../i18n/index.ts";
  import Icon from "./Icon.svelte";
  import { cn } from "./cn.ts";
  import { type UiTone, toneClass } from "./tone.ts";

  /**
   * A live number with its trend: label, value, a change in words and an arrow, a detail line and a sparkline of its own range. The
   * change is toned by whether it is good, not by its direction (design system: MetricTile).
   */
  let {
    label,
    icon,
    value,
    unit,
    delta,
    series,
    seriesLabel = t("ui.metricTile.recentValues"),
    color = 1,
    detail,
    class: klass = "",
  }: {
    label: string;
    icon: string;
    value: string;
    unit?: string;
    delta?: { text: string; tone: UiTone } | null;
    series?: number[] | null;
    seriesLabel?: string;
    color?: 1 | 2 | 3 | 4 | 5;
    detail?: string;
    class?: string;
  } = $props();

  const W = 200;
  const H = 44;
  const geometry = $derived.by(() => {
    if (!series || series.length < 2) return null;
    const lo = Math.min(...series);
    const hi = Math.max(...series);
    const points = series.map((v, i) => [3 + (i * (W - 6)) / (series.length - 1), hi === lo ? H / 2 : H - 3 - ((v - lo) / (hi - lo)) * (H - 6)] as const);
    const line = points.map(([x, y]) => `${x.toFixed(1)} ${y.toFixed(1)}`).join(" L");
    return { line: `M${line}`, area: `M${line} L${points[points.length - 1][0].toFixed(1)} ${H} L${points[0][0].toFixed(1)} ${H} Z` };
  });
  const SERIES = { 1: "text-series-1", 2: "text-series-2", 3: "text-series-3", 4: "text-series-4", 5: "text-series-5" } as const;
</script>

<div class={cn("flex min-w-0 flex-col gap-0.5 rounded-2xl border border-line bg-panel px-4 pt-3.5 pb-2.5 shadow-card", klass)}>
  <div class="flex items-center gap-2 text-xs text-muted">
    <Icon name={icon} size={13} class={SERIES[color]} />{label}
    {#if delta}<span class={cn("ml-auto text-xs font-semibold", toneClass(delta.tone, "text"))}>{delta.text}</span>{/if}
  </div>
  <div class="text-3xl font-semibold tracking-tight tabular-nums">{value}{#if unit}<small class="ml-0.5 text-base font-medium text-muted">{unit}</small>{/if}</div>
  {#if detail}<div class="text-xs text-muted">{detail}</div>{/if}
  {#if geometry}
    <svg class={cn("block h-11 w-full overflow-visible", SERIES[color])} viewBox="0 0 {W} {H}" preserveAspectRatio="none" role="img" aria-label="{seriesLabel}: {series?.join(', ')}">
      <path d={geometry.area} fill="currentColor" fill-opacity="0.14" stroke="none" />
      <path d={geometry.line} fill="none" stroke="currentColor" stroke-width="1.75" stroke-linejoin="round" stroke-linecap="round" vector-effect="non-scaling-stroke" />
    </svg>
  {/if}
</div>
