<script lang="ts">
  import { t } from "../i18n/index.ts";
  import type { Snippet } from "svelte";

  import Badge from "./Badge.svelte";
  import Icon from "./Icon.svelte";
  import Sparkbar from "./Sparkbar.svelte";
  import { cn } from "./cn.ts";
  import type { UiTone } from "./tone.ts";

  /**
   * One task (design system: TaskCard): its key, priority, title, progress, due date, what it waits for and who has it. A late due
   * date turns `err` and says "Today" or "2 days late". The priority is a flag, a word and a tone, never a tone alone.
   */
  let {
    taskKey,
    title,
    priority,
    due = null,
    progress = null,
    deps = 0,
    deliverables = 0,
    pull = null,
    status = null,
    selected = false,
    dragging = false,
    onopen,
    assignees,
    class: klass = "",
  }: {
    taskKey: string;
    title: string;
    priority: { label: string; text: string };
    due?: { text: string; late: boolean } | null;
    /** Shown as a bar while the task is going; leave out when it says nothing. */
    progress?: number | null;
    deps?: number;
    deliverables?: number;
    /** A pull request on the task, as its label says it. */
    pull?: string | null;
    status?: { text: string; tone: UiTone; live: boolean } | null;
    selected?: boolean;
    dragging?: boolean;
    onopen?: () => void;
    assignees?: Snippet;
    class?: string;
  } = $props();
</script>

<button
  type="button"
  onclick={onopen}
  aria-label="{taskKey}, {priority.label} priority, {title}"
  class={cn(
    "flex w-full flex-col gap-2 rounded-xl border bg-panel px-3 py-2.5 text-left shadow-1 transition duration-150 ease-out outline-none focus-visible:ring-2 focus-visible:ring-accent/60",
    dragging ? "-rotate-[1.5deg] border-accent shadow-3" : selected ? "border-accent shadow-2" : "border-line hover:-translate-y-px hover:shadow-2",
    klass,
  )}
>
  <span class="mono flex items-center gap-1.5 text-2xs font-medium text-faint">
    <span>{taskKey}</span>
    <span class={cn("inline-flex items-center gap-1 font-sans text-xs font-semibold", priority.text)}><Icon name="flag" size={11} />{priority.label}</span>
  </span>
  <span class="leading-snug font-medium">{title}</span>
  {#if progress !== null && progress > 0 && progress < 100}
    <span class="flex items-center gap-2">
      <Sparkbar value={progress} max={100} label={t("common.progress")} class="w-full flex-1" />
      <span class="tnum text-xs text-muted">{progress}%</span>
    </span>
  {/if}
  <span class="flex flex-wrap items-center gap-x-2 gap-y-1.5 text-xs whitespace-nowrap text-muted">
    {#if due}
      <span class={cn("inline-flex items-center gap-1", due.late && "font-medium text-err")}><Icon name={due.late ? "clock" : "calendar"} size={12} />{due.text}</span>
    {/if}
    {#if pull}
      <span class="inline-flex items-center gap-1"><Icon name="git-branch" size={12} />{pull}</span>
    {/if}
    {#if deps > 0}
      <span class="inline-flex items-center gap-1" aria-label="{deps} dependencies"><Icon name="link" size={12} />{deps}</span>
    {/if}
    {#if deliverables > 0}
      <span class="inline-flex items-center gap-1"><Icon name="file-text" size={12} />{deliverables} {deliverables === 1 ? "deliverable" : "deliverables"}</span>
    {/if}
    {#if status}<Badge tone={status.tone} live={status.live} class="h-4.5">{status.text}</Badge>{/if}
    {#if assignees}<span class="ml-auto inline-flex">{@render assignees()}</span>{/if}
  </span>
</button>
