<script lang="ts">
  import { fmt, t } from "../i18n/index.ts";
  import type { Snippet } from "svelte";

  import Avatar from "./Avatar.svelte";
  import Badge from "./Badge.svelte";
  import Icon from "./Icon.svelte";
  import Markdown from "./Markdown.svelte";
  import Sparkbar from "./Sparkbar.svelte";
  import { cn } from "./cn.ts";
  import type { UiTone } from "./tone.ts";

  export type ChildRow = {
    id: string;
    agent: string;
    name: string;
    emoji: string;
    color: string;
    task: string;
    state: "starting" | "working" | "waiting" | "completed" | "failed" | "cancelled";
    steps: number;
    cost_micros: number;
    /** What it answered, once it has. */
    answer?: string | null;
    /** Why it did not complete. */
    error?: string | null;
    /** The text it is writing right now. */
    live?: string | null;
  };

  /** One call to `agents.delegate`: who was asked, what each is doing, and how it went. Each row opens to its live text or its answer. */
  let {
    children: rows,
    open = $bindable(true),
    onopen,
    row: rowExtra,
    class: klass = "",
  }: { children: ChildRow[]; open?: boolean; onopen?: (id: string) => void; row?: Snippet<[ChildRow]>; class?: string } = $props();

  const LOOK: Record<ChildRow["state"], { tone: UiTone; text: string; live: boolean }> = {
    starting: { tone: "muted", text: t("ui.delegationCard.starting"), live: true },
    working: { tone: "info", text: t("common.working"), live: true },
    waiting: { tone: "warn", text: t("common.needsYou"), live: false },
    completed: { tone: "ok", text: t("common.done"), live: false },
    failed: { tone: "err", text: t("common.failed"), live: false },
    cancelled: { tone: "muted", text: t("common.stopped"), live: false },
  };

  let expanded = $state<Record<string, boolean>>({});
  const total = $derived(rows.reduce((n, r) => n + r.cost_micros, 0));
  const settled = $derived(rows.filter((r) => ["completed", "failed", "cancelled"].includes(r.state)).length);
  const overall = $derived<UiTone>(rows.some((r) => r.state === "waiting") ? "warn" : settled < rows.length ? "info" : rows.some((r) => r.state === "failed") ? "err" : "ok");
  const panel = $props.id();
  const money = (micros: number) => (micros === 0 ? t("format.free") : fmt.usd4(micros));
  const isOpen = (r: ChildRow) => expanded[r.id] ?? (r.state === "working" || r.state === "waiting");
</script>

<section class={cn("overflow-hidden rounded-2xl border bg-panel-2", overall === "warn" ? "border-warn/50" : "border-line", klass)} aria-label={t("ui.delegationCard.workHandedToOther")}>
  <button type="button" class="flex w-full items-center gap-3 px-3.5 py-2.5 text-left transition hover:bg-hover" aria-expanded={open} aria-controls={open ? panel : undefined} onclick={() => (open = !open)}>
    <span class="grid size-7 shrink-0 place-items-center rounded-lg bg-accent-soft text-accent"><Icon name="branch" size={15} /></span>
    <span class="min-w-0 flex-1">
      <span class="block text-md font-semibold">{t("ui.delegationCard.askedAgents", { count: rows.length })}</span>
      <span class="block text-xs text-muted">{t("ui.delegationCard.ofFinishedSoFar", { settled, length: rows.length, value: money(total) })}</span>
    </span>
    <Badge tone={overall} live={settled < rows.length && overall === "info"}>{settled < rows.length ? (overall === "warn" ? t("common.needsYou") : t("common.working")) : overall === "err" ? t("ui.delegationCard.someFailed") : t("ui.delegationCard.allDone")}</Badge>
    <Icon name={open ? "chevron-down" : "chevron-right"} size={14} class="text-faint" />
  </button>
  {#if open}
    <ul id={panel} class="divide-y divide-line-soft border-t border-line-soft">
      {#each rows as r (r.id)}
        {@const look = LOOK[r.state]}
        <li class="px-3.5 py-2.5">
          <div class="flex items-center gap-3">
            <Avatar emoji={r.emoji} color={r.color} size="sm" />
            <button type="button" class="min-w-0 flex-1 text-left" aria-expanded={isOpen(r)} onclick={() => (expanded[r.id] = !isOpen(r))}>
              <span class="flex items-baseline gap-2"><span class="text-md font-medium">{r.name}</span><span class="mono text-xs text-faint">{r.agent}</span></span>
              <span class="block truncate text-sm text-muted">{r.task}</span>
            </button>
            <span class="hidden items-center gap-2 text-xs text-muted tabular-nums sm:flex"><Sparkbar value={r.cost_micros} max={total} label={t("common.shareOfTheCost", { name: r.name })} />{r.steps} step{r.steps === 1 ? "" : "s"}</span>
            <Badge tone={look.tone} live={look.live} dot>{look.text}</Badge>
            {#if onopen}<button type="button" class="grid size-7 place-items-center rounded-lg text-faint transition hover:bg-hover hover:text-fg" aria-label={t("ui.delegationCard.openTheRunOf", { name: r.name })} onclick={() => onopen?.(r.id)}><Icon name="external" size={14} /></button>{/if}
          </div>
          {#if isOpen(r)}
            <div class="mt-2 ml-10 space-y-2">
              {#if r.error}
                <p class="rounded-lg border border-err/35 bg-err/8 px-3 py-2 text-sm text-err" role="alert">{r.error}</p>
              {:else if r.answer}
                <div class="rounded-xl border border-line bg-panel px-3 py-2"><Markdown text={r.answer} /></div>
              {:else if r.live}
                <div class="stream-caret rounded-xl border border-line bg-panel px-3 py-2"><Markdown text={r.live} /></div>
              {:else}
                <p class="text-sm text-muted">{t("ui.delegationCard.workingOnIt")}</p>
              {/if}
              {@render rowExtra?.(r)}
            </div>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>
