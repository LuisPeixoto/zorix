<script lang="ts">
  import Icon from "./Icon.svelte";
  import { cn } from "./cn.ts";

  /** How a workflow starts, shown on its row, in lists and in the canvas header. The label is the human schedule or source. */
  let { kind, label, class: klass = "" }: { kind: "schedule" | "webhook" | "event" | "manual" | "workflow"; label: string; class?: string } = $props();

  const ICON = { schedule: "calendar", webhook: "webhook", event: "zap", manual: "play", workflow: "workflow" } as const;
</script>

<span class={cn("inline-flex h-6 max-w-full items-center gap-1.5 rounded-full bg-node-trigger/12 pr-2.5 pl-1 text-xs font-semibold text-node-trigger", klass)}>
  <span class="grid size-4.5 shrink-0 place-items-center rounded-full bg-node-trigger text-bg"><Icon name={ICON[kind]} size={12} stroke={2.2} /></span>
  <span class="truncate">{label}</span>
</span>
