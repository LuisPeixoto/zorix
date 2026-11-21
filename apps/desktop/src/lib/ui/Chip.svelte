<script lang="ts">
  import type { Snippet } from "svelte";

  import Icon from "./Icon.svelte";
  import { cn } from "./cn.ts";

  let {
    selected = false,
    onclick,
    onremove,
    class: klass = "",
    children,
  }: { selected?: boolean; onclick?: () => void; onremove?: () => void; class?: string; children: Snippet } = $props();
</script>

<span
  class={cn(
    "inline-flex h-6.5 items-center rounded-full border text-sm font-medium transition duration-150",
    selected ? "border-accent/50 bg-accent-soft text-accent" : "border-line bg-panel text-fg hover:border-line-strong",
    klass,
  )}
>
  <button type="button" {onclick} aria-pressed={onclick ? selected : undefined} class={cn("h-full rounded-full px-2.5", onremove && "pr-1")}>{@render children()}</button>
  {#if onremove}
    <button type="button" onclick={onremove} aria-label="Remove" class="mr-1 grid size-5 place-items-center rounded-full hover:bg-hover"><Icon name="x" size={12} /></button>
  {/if}
</span>
