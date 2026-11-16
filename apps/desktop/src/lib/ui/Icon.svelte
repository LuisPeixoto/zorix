<script lang="ts">
  import { ICONS } from "./icons.ts";
  import { cn } from "./cn.ts";

  let { name, size = 16, stroke = 1.8, class: klass = "" }: { name: string; size?: number; stroke?: number; class?: string } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width={stroke}
  stroke-linecap="round"
  stroke-linejoin="round"
  class={cn("shrink-0", klass)}
  aria-hidden="true"
>
  <!-- eslint-disable-next-line svelte/no-at-html-tags -- static markup from icons.ts -->
  {@html ICONS[name] ?? ICONS.info}
</svg>
