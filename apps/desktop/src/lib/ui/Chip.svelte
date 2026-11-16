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
    "inline-flex h-7 items-center rounded-full border text-xs transition duration-150",
    selected ? "border-accent/50 bg-accent-soft text-accent" : "border-line bg-panel-2 text-muted hover:border-line-strong hover:text-fg",
    klass,
  )}
>
  <button type="button" {onclick} aria-pressed={onclick ? selected : undefined} class={cn("h-full rounded-full px-2.5", onremove && "pr-1")}>{@render children()}</button>
  {#if onremove}
    <button type="button" onclick={onremove} aria-label="Remove" class="mr-1 grid size-5 place-items-center rounded-full hover:bg-hover"><Icon name="x" size={12} /></button>
  {/if}
</span>
