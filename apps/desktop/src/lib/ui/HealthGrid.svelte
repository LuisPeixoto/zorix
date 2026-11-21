<script lang="ts">
  import BrandLogo from "./BrandLogo.svelte";
  import Icon from "./Icon.svelte";
  import StatusPill from "./StatusPill.svelte";
  import { cn } from "./cn.ts";
  import { type HealthItem, describeDay, lastDay, sortFailingFirst } from "./health.ts";

  /**
   * The health of every provider, integration and server at a glance: a mark, a name, a state, 24 hours of history and one figure.
   * Failing cells come first. A cell with `onopen` is a button (a failing cell opens its logs).
   */
  let { items, onopen, class: klass = "" }: { items: HealthItem[]; onopen?: (item: HealthItem) => void; class?: string } = $props();

  const cells = $derived(sortFailingFirst(items));
  const cell = "flex w-full min-w-0 flex-col gap-2 rounded-xl border border-line bg-panel px-3 py-2.5 text-left";
  const bar = { ok: "bg-ok/70", warn: "bg-warn", err: "bg-err", none: "bg-hover" };
</script>

{#snippet body(item: HealthItem)}
  <span class="flex items-center gap-2 text-base font-medium">
    {#if item.brand}<BrandLogo name={item.brand} size={22} />{:else}<Icon name={item.icon ?? "plug"} size={15} class="text-muted" />{/if}
    <span class="min-w-0 flex-1 truncate">{item.name}</span>
    <StatusPill state={item.state} />
  </span>
  <span class="flex h-4.5 gap-0.5" role="img" aria-label={describeDay(item.history)}>
    {#each lastDay(item.history) as kind, hour (hour)}<i class={cn("flex-1 rounded-sm", bar[kind])}></i>{/each}
  </span>
  <span class="truncate text-xs text-muted">{item.note}</span>
{/snippet}

<ul class={cn("grid grid-cols-[repeat(auto-fill,minmax(200px,1fr))] gap-2", klass)} aria-label="Health">
  {#each cells as item (item.id)}
    <li class="min-w-0">
      {#if onopen}
        <button type="button" onclick={() => onopen(item)} class={cn(cell, "transition duration-150 hover:border-line-strong hover:bg-hover")}>{@render body(item)}</button>
      {:else}
        <div class={cell}>{@render body(item)}</div>
      {/if}
    </li>
  {/each}
</ul>
