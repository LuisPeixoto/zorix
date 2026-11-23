<script lang="ts">
  import { t } from "../i18n/index.ts";
  import { cn } from "./cn.ts";

  type Tab = { value: string; label: string; count?: number };
  let { tabs, value = $bindable(), label = t("ui.tabs.sections"), class: klass = "" }: { tabs: Tab[]; value: string; label?: string; class?: string } = $props();
</script>

<div role="tablist" aria-label={label} class={cn("flex flex-wrap gap-x-4.5 border-b border-line", klass)}>
  {#each tabs as tab (tab.value)}
    <button
      type="button"
      role="tab"
      aria-selected={value === tab.value}
      onclick={() => (value = tab.value)}
      class={cn(
        "relative inline-flex items-center gap-1.5 pt-2 pb-2.5 text-base font-medium transition duration-150",
        value === tab.value ? "text-fg after:absolute after:inset-x-0 after:-bottom-px after:h-0.5 after:rounded-sm after:bg-accent" : "text-muted hover:text-fg",
      )}
    >
      {tab.label}
      {#if tab.count !== undefined}<span class="mono rounded-full bg-hover px-1.5 text-2xs text-muted tabular-nums">{tab.count}</span>{/if}
    </button>
  {/each}
</div>
