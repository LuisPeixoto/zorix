<script lang="ts">
  import type { Snippet } from "svelte";

  import { Icon, cn } from "../../ui/index.ts";

  let {
    icon,
    active = false,
    count,
    attention = false,
    onclick,
    children,
  }: { icon: string; active?: boolean; count?: number; attention?: boolean; onclick: () => void; children: Snippet } = $props();
</script>

<button
  type="button"
  {onclick}
  aria-current={active ? "page" : undefined}
  class={cn(
    "group relative flex h-8 w-full items-center gap-2.5 rounded-lg px-2.5 text-md transition duration-150",
    active ? "bg-accent-soft font-medium text-fg" : "text-fg/80 hover:bg-hover hover:text-fg",
  )}
>
  {#if active}<span class="absolute top-1.5 bottom-1.5 left-0 w-[3px] rounded-full bg-accent" aria-hidden="true"></span>{/if}
  <Icon name={icon} size={15} class={active ? "text-accent" : "text-faint group-hover:text-muted"} />
  <span class="min-w-0 flex-1 truncate text-left">{@render children()}</span>
  {#if attention}<span class="size-1.5 rounded-full bg-warn" role="img" aria-label="Needs attention"></span>{/if}
  {#if count !== undefined && count > 0}<span class="tnum rounded-full bg-hover px-1.5 text-2xs text-muted">{count}</span>{/if}
</button>
