<script lang="ts">
  import { t } from "../i18n/index.ts";
  import type { Snippet } from "svelte";

  import type { TaskView } from "../api.ts";
  import { type Timeline, dependencyArrows, dayLabel, dayText } from "../project.ts";
  import { cn } from "./cn.ts";
  import type { UiTone } from "./tone.ts";

  /**
   * A project's tasks across time (design system: the timeline grid). Each task is a bar from its start to its due date, filled by
   * its progress and tinted by priority or state; the bar keeps the percentage in words. A line marks today. An arrow runs from the
   * end of a task to the start of the one that waits for it, and is red when that one starts before the first is due.
   */
  let {
    timeline,
    avatar,
    onopen,
  }: {
    timeline: Timeline;
    avatar?: Snippet<[view: TaskView]>;
    onopen: (key: string) => void;
  } = $props();

  const ROW = 44;
  // Written out in full so Tailwind can see them.
  const BAR: Record<UiTone, { box: string; fill: string }> = {
    ok: { box: t("ui.timeline.borderOk45Bg"), fill: "bg-ok/22" },
    warn: { box: t("ui.timeline.borderWarn45Bg"), fill: "bg-warn/22" },
    err: { box: t("ui.timeline.borderErr45Bg"), fill: "bg-err/22" },
    info: { box: t("ui.timeline.borderInfo45Bg"), fill: "bg-info/22" },
    muted: { box: t("ui.timeline.borderMuted45Bg"), fill: "bg-muted/22" },
    accent: { box: t("ui.timeline.borderAccent45Bg"), fill: "bg-accent/22" },
  };

  let laneWidth = $state(0);
  const arrows = $derived(dependencyArrows(timeline.bars, laneWidth, ROW));
  const columns = $derived(timeline.columns.map((c) => `${c.days}fr`).join(" "));
</script>

<div class="relative overflow-hidden rounded-2xl border border-line bg-panel shadow-card">
  <div class="grid grid-cols-[260px_minmax(0,1fr)] border-b border-line-soft bg-panel-2">
    <span class="px-3.5 py-2.5 text-2xs font-semibold tracking-wider text-faint uppercase">{t("common.task")}</span>
    <div class="grid" style:grid-template-columns={columns}>
      {#each timeline.columns as column (column.start)}
        <span class="border-l border-line-soft px-2.5 py-2.5 text-2xs font-semibold tracking-wider text-faint uppercase" title={dayLabel(dayText(column.start))}>{column.label}</span>
      {/each}
    </div>
  </div>

  <div class="relative">
    <!-- Columns, today and arrows sit over the rows, to the right of the labels. -->
    <div class="pointer-events-none absolute inset-y-0 right-0 left-65 grid" style:grid-template-columns={columns} aria-hidden="true">
      {#each timeline.columns as column (column.start)}<span class="border-l border-line-soft"></span>{/each}
    </div>
    <div class="pointer-events-none absolute inset-y-0 right-0 left-65 z-10" bind:clientWidth={laneWidth} aria-hidden="true">
      {#if timeline.today !== null}
        <span class="absolute inset-y-0 w-0.5 bg-accent" style:left="{timeline.today * 100}%">
          <span class="absolute bottom-2 left-1/2 -translate-x-1/2 rounded-full bg-accent px-2 py-0.5 text-2xs font-semibold whitespace-nowrap text-accent-fg shadow-1">{t("ui.timeline.today")}</span>
        </span>
      {/if}
      {#if laneWidth > 0 && arrows.length > 0}
        <svg class="absolute inset-0 overflow-visible" width={laneWidth} height={timeline.bars.length * ROW}>
          {#each arrows as arrow (arrow.from + arrow.to)}
            <path d={arrow.path} fill="none" stroke-width="1.5" class={arrow.late ? "stroke-err" : "stroke-faint"} />
            <polygon points={arrow.head} class={arrow.late ? "fill-err" : "fill-faint"} />
          {/each}
        </svg>
      {/if}
    </div>

    {#each timeline.bars as bar (bar.view.task.key)}
      {@const task = bar.view.task}
      <div class="grid h-11 grid-cols-[260px_minmax(0,1fr)] border-b border-line-soft last:border-b-0">
        <button type="button" class="flex min-w-0 items-center gap-2 px-3.5 text-left font-medium outline-none hover:bg-hover focus-visible:bg-hover" onclick={() => onopen(task.key)}>
          <span class="mono shrink-0 text-2xs whitespace-nowrap text-faint">{task.key}</span>
          <span class="truncate">{task.title}</span>
        </button>
        <div class="relative">
          <button
            type="button"
            onclick={() => onopen(task.key)}
            aria-label={t("ui.timeline.doneFromTo", { key: task.key, title: task.title, progress: task.progress, value: dayLabel(dayText(bar.start)), value2: dayLabel(dayText(bar.end)) })}
            class={cn("absolute top-2 z-0 flex h-6.5 items-center gap-1.5 overflow-hidden rounded-lg border px-2 text-xs font-medium whitespace-nowrap outline-none focus-visible:ring-2 focus-visible:ring-accent/60", BAR[bar.tone].box)}
            style:left="{bar.left * 100}%"
            style:width="max({bar.width * 100}%, 4.5rem)"
          >
            <span class={cn("absolute inset-y-0 left-0", BAR[bar.tone].fill)} style:width="{task.progress}%"></span>
            {#if avatar}<span class="relative inline-flex">{@render avatar(bar.view)}</span>{/if}
            <span class="tnum relative">{task.progress}%</span>
          </button>
        </div>
      </div>
    {/each}
    {#if timeline.bars.length === 0}
      <p class="px-4 py-8 text-center text-base text-muted">{t("ui.timeline.noTaskHasA")}</p>
    {/if}
  </div>
</div>
