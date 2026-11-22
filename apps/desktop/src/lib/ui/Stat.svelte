<script lang="ts">
  import Icon from "./Icon.svelte";
  import { cn } from "./cn.ts";

  let { label, value, detail, icon, compact = false, class: klass = "" }: { label: string; value: string | number; detail?: string; icon?: string; compact?: boolean; class?: string } = $props();
</script>

<div class={cn("rounded-2xl border border-line bg-panel shadow-raised", compact ? "px-3 py-2.5" : "px-4 py-3.5", klass)}>
  <div class="flex items-center gap-1.5 text-sm text-muted">
    {#if icon}<Icon name={icon} size={13} class="text-accent" />{/if}{label}
  </div>
  <div class={cn("mt-1 font-semibold tracking-tight tabular-nums", compact ? "text-lg" : "text-3xl")}>{value}</div>
  {#if detail}<div class="mt-0.5 text-xs text-muted">{detail}</div>{/if}
</div>
