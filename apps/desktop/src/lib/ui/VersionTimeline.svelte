<script lang="ts">
  import type { Snippet } from "svelte";

  import Badge from "./Badge.svelte";
  import { cn } from "./cn.ts";

  export type VersionItem = { version: number; hash: string; note?: string | null; at: string; current?: boolean };

  /** The versions of an agent, newest first, joined by a line. Pick one to compare or to go back to. */
  let {
    items,
    selected = null,
    onselect,
    actions,
    class: klass = "",
  }: { items: VersionItem[]; selected?: number | null; onselect?: (version: number) => void; actions?: Snippet<[VersionItem]>; class?: string } = $props();
</script>

<ol class={cn("relative", klass)} aria-label="Versions">
  {#each items as item, i (item.version)}
    <li class="relative flex gap-3 pb-1">
      <div class="flex w-5 shrink-0 flex-col items-center" aria-hidden="true">
        <span class={cn("mt-3.5 size-3 rounded-full border-2", item.current ? "border-accent bg-accent shadow-[0_0_0_4px_var(--accent-soft)]" : "border-line-strong bg-panel")}></span>
        {#if i < items.length - 1}<span class="mt-1 w-px flex-1 bg-line"></span>{/if}
      </div>
      <div
        class={cn(
          "mb-1 flex min-w-0 flex-1 items-center gap-3 rounded-xl border px-3 py-2.5 transition duration-150",
          selected === item.version ? "border-accent/50 bg-accent-soft" : "border-transparent hover:bg-hover",
        )}
      >
        <button type="button" class="min-w-0 flex-1 text-left" aria-pressed={onselect ? selected === item.version : undefined} disabled={!onselect} onclick={() => onselect?.(item.version)}>
          <span class="flex items-center gap-2">
            <span class="text-md font-semibold">v{item.version}</span>
            {#if item.current}<Badge tone="accent">Current</Badge>{/if}
            <span class="mono text-xs text-faint">{item.hash.slice(0, 8)}</span>
          </span>
          <span class="mt-0.5 block truncate text-sm text-muted">{item.note || "No note"}</span>
          <span class="block text-xs text-faint">{item.at}</span>
        </button>
        {#if actions}<div class="flex shrink-0 gap-1.5">{@render actions(item)}</div>{/if}
      </div>
    </li>
  {/each}
</ol>
