<script lang="ts">
  import type { Snippet } from "svelte";

  import { cn } from "./cn.ts";

  /** Avatars that overlap by a few pixels, each with a ring in the surface colour so the edges stay clear (design system: Avatar). */
  let { extra = 0, class: klass = "", children }: { extra?: number; class?: string; children: Snippet } = $props();
</script>

<span class={cn("inline-flex items-center -space-x-1.5 *:shrink-0 *:ring-2 *:ring-panel", klass)}>
  {@render children()}
  {#if extra > 0}
    <span class="inline-grid size-7 place-items-center rounded-full border border-line bg-panel-2 text-2xs font-semibold text-muted" aria-label="{extra} more">+{extra}</span>
  {/if}
</span>
