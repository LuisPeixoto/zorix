<script lang="ts">
  import { t } from "../i18n/index.ts";
  import Badge from "./Badge.svelte";
  import Icon from "./Icon.svelte";
  import { cn } from "./cn.ts";
  import type { UiTone } from "./tone.ts";
  import { NODE_BG, NODE_H, NODE_TEXT, NODE_TINT, NODE_W, type NodeClass, type NodeState } from "../workflow.ts";

  /**
   * One step of a workflow (design system: WorkflowNode). A 2px edge in the colour of its kind, an icon tile, the title and what
   * kind it is, and in and out ports. The state is a border, a halo or a badge: never only a colour.
   */
  let {
    id,
    cls,
    icon,
    title,
    kind,
    detail = "",
    state = "idle",
    selected = false,
    badge,
    x,
    y,
    readonly = false,
    connecting = false,
  }: {
    id: string;
    cls: NodeClass;
    icon: string;
    title: string;
    kind: string;
    detail?: string;
    state?: NodeState;
    selected?: boolean;
    /** What a run says about the step: "fired 08:00", "yes", "5 steps · $0.21", "Waiting 14 min". */
    badge?: { text: string; tone: UiTone; live?: boolean };
    x: number;
    y: number;
    readonly?: boolean;
    /** A connection is being dragged: the in port lights up as a target. */
    connecting?: boolean;
  } = $props();

  const STATE_WORD: Record<NodeState, string> = { idle: "", running: "running", waiting: t("ui.workflowNode.waitingForAPerson"), done: "done", failed: "failed", skipped: "skipped" };
</script>

<div
  data-node={id}
  role="button"
  tabindex="0"
  aria-pressed={selected}
  aria-label="{title}. {kind}.{STATE_WORD[state] ? ` ${STATE_WORD[state]}.` : ''}"
  class={cn(
    "absolute overflow-visible rounded-xl border border-line bg-panel text-xs shadow-raised outline-none select-none focus-visible:ring-2 focus-visible:ring-accent/40",
    readonly ? "cursor-pointer" : "cursor-grab active:cursor-grabbing",
    state === "running" && "border-info/55",
    state === "waiting" && "border-warn/55 ring-4 ring-warn/10",
    state === "done" && "border-ok/40",
    state === "failed" && "border-err",
    state === "skipped" && "border-dashed opacity-60 shadow-none",
    selected && "border-accent ring-3 ring-accent/25",
  )}
  style:left="{x}px"
  style:top="{y}px"
  style:width="{NODE_W}px"
  style:height="{NODE_H}px"
>
  <span class={cn("absolute inset-x-0 top-0 h-0.5 rounded-t-xl opacity-90", NODE_BG[cls])} aria-hidden="true"></span>
  <span data-port="in" class={cn("absolute -top-1.5 left-1/2 -ml-1.25 size-2.5 rounded-full border-2 bg-panel", connecting ? "border-accent" : "border-line-strong")} aria-hidden="true"></span>
  <div class="flex items-center gap-2 px-2.5 pt-2 pb-1">
    <span class={cn("grid size-6 shrink-0 place-items-center rounded-md", NODE_TINT[cls], NODE_TEXT[cls])}><Icon name={icon} size={14} /></span>
    <span class="min-w-0">
      <span class="block truncate text-sm font-semibold">{title}</span>
      <span class="block truncate text-2xs text-muted">{kind}</span>
    </span>
  </div>
  <div class="flex items-center gap-1.5 truncate pr-2.5 pl-10.5 text-xs text-muted">
    {#if badge}
      <Badge tone={badge.tone} live={badge.live} class="h-4.5 max-w-full truncate">{badge.text}</Badge>
    {:else if detail}
      <span class="truncate">{detail}</span>
    {/if}
  </div>
  <span data-port="out" class={cn("absolute -bottom-1.5 left-1/2 -ml-1.25 size-2.5 rounded-full border-2 border-line-strong bg-panel", !readonly && "cursor-crosshair hover:border-accent")} aria-hidden="true"></span>
</div>
