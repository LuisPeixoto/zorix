<script lang="ts">
  import { BRANDS } from "./brands.ts";
  import Icon from "./Icon.svelte";
  import { cn } from "./cn.ts";

  /** A service's mark on a quiet tile. Services without a mark (or a custom server) get a plug. */
  let {
    name,
    size = 36,
    class: klass = "",
  }: { name?: string; size?: number; class?: string } = $props();

  // Catalogue entries that are not brands use a plain icon.
  const GENERIC: Record<string, string> = { folder: "folder", globe: "globe", playwright: "monitor", deepwiki: "book", server: "server" };
  const path = $derived(name ? BRANDS[name] : undefined);
  const generic = $derived(name ? (GENERIC[name] ?? "plug") : "plug");
</script>

<span
  class={cn("grid shrink-0 place-items-center rounded-[10px] border border-line bg-panel-2 text-fg shadow-1 raised", klass)}
  style="width:{size}px;height:{size}px"
  aria-hidden="true"
>
  {#if path}
    <svg width={size * 0.5} height={size * 0.5} viewBox="0 0 24 24" fill="currentColor"><path d={path} /></svg>
  {:else}
    <Icon name={generic} size={size * 0.5} class="text-muted" />
  {/if}
</span>
