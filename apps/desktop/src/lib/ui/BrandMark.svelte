<script lang="ts">
  import Icon from "./Icon.svelte";
  import { cn } from "./cn.ts";

  /**
   * The ZORIX mark: an icon (`layers` by default) on a tile filled with the brand gradient. `sm` is 28 px and sits beside the wordmark in the
   * toolbar, `lg` is 56 px for empty states and the About screen. Pass `wordmark` to set ZORIX beside the small tile. Never recolour the tile.
   */
  let {
    size = "sm",
    icon = "layers",
    wordmark = false,
    class: klass = "",
  }: { size?: "sm" | "lg"; icon?: string; wordmark?: boolean; class?: string } = $props();
</script>

<span class={cn("inline-flex items-center gap-2.5", klass)}>
  <span
    class={cn("brand-gradient brand-tile relative grid shrink-0 place-items-center", size === "sm" ? "size-7 rounded-lg" : "size-14 rounded-2xl")}
    aria-hidden={wordmark ? "true" : undefined}
  >
    <Icon name={icon} size={size === "sm" ? 15 : 26} stroke={size === "sm" ? 2 : 1.8} />
  </span>
  {#if wordmark}<span class="text-lg font-semibold tracking-tight">ZORIX</span>{/if}
</span>
