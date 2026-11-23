<script lang="ts">
  import { t } from "../i18n/index.ts";
  import { cn } from "./cn.ts";

  export type Level = "suggest" | "ask" | "notify" | "budgeted" | "full";
  const LEVELS: { id: Level; label: string; blurb: string }[] = [
    { id: "suggest", label: t("ui.autonomyDial.suggest"), blurb: t("ui.autonomyDial.proposesAPersonActs") },
    { id: "ask", label: t("ui.autonomyDial.ask"), blurb: t("ui.autonomyDial.actsAfterEachApproval") },
    { id: "notify", label: t("ui.autonomyDial.notify"), blurb: t("ui.autonomyDial.actsThenReports") },
    { id: "budgeted", label: t("ui.autonomyDial.budgeted"), blurb: t("ui.autonomyDial.actsWithinLimits") },
    { id: "full", label: t("ui.autonomyDial.full"), blurb: t("ui.autonomyDial.actsReportsWeekly") },
  ];

  /**
   * How much a worker may do without asking: five levels. The current one is a raised tile and the bars fill up to it. A level above
   * `max` cannot be chosen (a worker cannot be freer than who it reports to). The dial never overrides policy: a tool that always
   * asks still asks (design system: AutonomyDial).
   */
  let {
    level = $bindable("ask"),
    max = "full",
    disabled = false,
    label = t("common.autonomy"),
    onchange,
    class: klass = "",
  }: { level?: Level; max?: Level; disabled?: boolean; label?: string; onchange?: (level: Level) => void; class?: string } = $props();

  const index = $derived(LEVELS.findIndex((l) => l.id === level));
  const top = $derived(LEVELS.findIndex((l) => l.id === max));

  function pick(i: number) {
    if (disabled || i > top || i < 0 || LEVELS[i].id === level) return;
    level = LEVELS[i].id;
    onchange?.(level);
  }

  function onkey(event: KeyboardEvent) {
    const step = event.key === "ArrowRight" || event.key === "ArrowDown" ? 1 : event.key === "ArrowLeft" || event.key === "ArrowUp" ? -1 : 0;
    if (!step) return;
    event.preventDefault();
    const next = Math.min(top, Math.max(0, index + step));
    pick(next);
    (event.currentTarget as HTMLElement).querySelectorAll<HTMLButtonElement>("button")[next]?.focus();
  }
</script>

<div role="radiogroup" aria-label={label} tabindex="-1" onkeydown={onkey} class={cn("grid grid-cols-5 gap-1 rounded-xl bg-hover p-1", klass)}>
  {#each LEVELS as l, i (l.id)}
    {@const locked = i > top}
    <button
      type="button"
      role="radio"
      aria-checked={i === index}
      tabindex={i === index ? 0 : -1}
      disabled={disabled || locked}
      title={locked ? t("ui.autonomyDial.aboveTheOneIt") : l.blurb}
      onclick={() => pick(i)}
      class={cn(
        "flex flex-col items-center gap-1 rounded-lg px-1 py-2 text-xs font-medium outline-none transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-accent/60",
        i === index ? "bg-panel text-fg shadow-raised" : "text-muted hover:text-fg",
        (locked || disabled) && "cursor-not-allowed opacity-55 hover:text-muted",
      )}
    >
      <i class={cn("block h-1 w-full max-w-9 rounded-full", i <= index ? "bg-accent" : "bg-line-strong")}></i>
      {l.label}
    </button>
  {/each}
</div>
