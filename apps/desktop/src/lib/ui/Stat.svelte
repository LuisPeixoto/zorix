<script lang="ts">
  import Icon from "./Icon.svelte";
  import { cn } from "./cn.ts";

  let { label, value, detail, icon, class: klass = "" }: { label: string; value: string | number; detail?: string; icon?: string; class?: string } = $props();
</script>

<div class={cn("rounded-2xl border border-line bg-panel px-4 py-3 shadow-card", klass)}>
  <div class="flex items-center gap-1.5 text-xs font-medium text-muted">
    {#if icon}<Icon name={icon} size={13} class="text-accent" />{/if}{label}
  </div>
  <div class="mt-1 text-3xl font-semibold tracking-tight tabular-nums">{value}</div>
  {#if detail}<div class="mt-0.5 text-xs text-faint">{detail}</div>{/if}
</div>
