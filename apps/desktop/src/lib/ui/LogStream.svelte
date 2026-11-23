<script lang="ts">
  import { t } from "../i18n/index.ts";
  import { lineTime } from "../workforce.ts";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import { cn } from "./cn.ts";
  import { toneClass } from "./tone.ts";

  export interface LogLine {
    id: string;
    at: number;
    level: "OK" | "INFO" | "WAIT" | "WARN" | "ERR";
    source: string;
    text: string;
    /** The run the line came from, when it has one. */
    run?: string | null;
  }

  /**
   * Events as they happen, one line each: time, level, source and message, in mono at 11.5px. A line opens its run. A live stream can
   * pause, and a paused stream says how many lines arrived since (design system: LogStream). Secrets are redacted before a line is stored.
   */
  let {
    lines,
    now,
    live = true,
    scope = t("ui.logStream.allSources"),
    empty = t("ui.logStream.nothingYet"),
    onopen,
    class: klass = "",
  }: { lines: LogLine[]; now: number; live?: boolean; scope?: string; empty?: string; onopen?: (line: LogLine) => void; class?: string } = $props();

  let paused = $state(false);
  let frozen = $state<LogLine[]>([]);
  let warnOnly = $state(false);

  const shown = $derived(paused ? frozen : lines);
  const visible = $derived(warnOnly ? shown.filter((l) => l.level === "WAIT" || l.level === "WARN" || l.level === "ERR") : shown);
  const arrived = $derived(paused ? Math.max(0, lines.length - frozen.length) : 0);
  const TONE = { OK: "ok", INFO: "info", WAIT: "warn", WARN: "warn", ERR: "err" } as const;

  const ROW =
    t("ui.logStream.gridWFullGrid");

  function toggle() {
    if (!paused) frozen = lines;
    paused = !paused;
  }
</script>

{#snippet cells(line: LogLine)}
  <span class="tnum text-faint">{lineTime(line.at, now)}</span>
  <span class={cn("font-semibold", toneClass(TONE[line.level], "text"))}>{line.level}</span>
  <span class="truncate text-accent">{line.source}</span>
  <span class="truncate">{line.text}</span>
{/snippet}

<div class={cn("overflow-hidden rounded-xl border border-line bg-panel", klass)}>
  <div class="flex items-center gap-2 border-b border-line-soft bg-panel-2 px-3 py-2 text-xs font-medium">
    {#if live && !paused}<span class="size-2 shrink-0 animate-pulse rounded-full bg-ok" aria-hidden="true"></span>{t("ui.logStream.live")}{:else if paused}<span class="size-2 shrink-0 rounded-full bg-muted" aria-hidden="true"></span>{t("common.paused")}{/if}
    <span class="font-normal text-muted tnum">· {scope} · {visible.length} of {shown.length}</span>
    {#if arrived > 0}<span class="font-normal text-muted tnum" aria-live="polite">{t("ui.logStream.newSince", { arrived })}</span>{/if}
    <span class="ml-auto flex items-center gap-1.5">
      <button
        type="button"
        aria-pressed={warnOnly}
        onclick={() => (warnOnly = !warnOnly)}
        class="inline-flex h-6 items-center gap-1 rounded-full border border-line bg-panel px-2.5 text-xs font-medium text-muted outline-none hover:text-fg focus-visible:ring-2 focus-visible:ring-accent/60 {warnOnly ? 'border-accent! text-fg' : ''}"
      >
        <Icon name="filter" size={12} />{warnOnly ? t("ui.logStream.warnAndAbove") : t("ui.logStream.allLevels")}
      </button>
      <IconButton icon={paused ? "play" : "pause"} label={paused ? t("ui.logStream.resumeTheStream") : t("ui.logStream.pauseTheStream")} size="sm" onclick={toggle} />
    </span>
  </div>
  {#if visible.length === 0}
    <p class="px-4 py-6 text-center text-sm text-muted">{empty}</p>
  {:else}
    <ul class="mono py-1.5 text-xs leading-[1.75]" aria-label={t("ui.logStream.eventsNewestFirst")}>
      {#each visible as line (line.id)}
        <li>
          {#if onopen}
            <button type="button" onclick={() => onopen(line)} class={ROW + " cursor-pointer text-left"}>
              {@render cells(line)}
            </button>
          {:else}
            <div class={ROW}>{@render cells(line)}</div>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>
