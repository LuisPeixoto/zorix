<script lang="ts">
  import { cn } from "./cn.ts";

  type Tab = { value: string; label: string; count?: number };
  let { tabs, value = $bindable(), label = "Sections", class: klass = "" }: { tabs: Tab[]; value: string; label?: string; class?: string } = $props();
</script>

<div role="tablist" aria-label={label} class={cn("flex flex-wrap gap-1", klass)}>
  {#each tabs as tab (tab.value)}
    <button
      type="button"
      role="tab"
      aria-selected={value === tab.value}
      onclick={() => (value = tab.value)}
      class={cn(
        "inline-flex h-7 items-center gap-1.5 rounded-lg px-2.5 text-sm font-medium transition duration-150",
        value === tab.value ? "bg-accent-soft text-accent" : "text-muted hover:bg-hover hover:text-fg",
      )}
    >
      {tab.label}
      {#if tab.count !== undefined}<span class="mono rounded-full bg-hover px-1.5 text-2xs text-muted tabular-nums">{tab.count}</span>{/if}
    </button>
  {/each}
</div>
