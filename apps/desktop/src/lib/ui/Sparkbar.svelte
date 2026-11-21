<script lang="ts">
  import { cn } from "./cn.ts";
  import { type UiTone, toneClass } from "./tone.ts";

  /** A small bar for a share of something, with the number said in words for anyone who cannot see the bar. */
  let {
    value,
    max = 1,
    label,
    tone = "accent",
    class: klass = "",
  }: { value: number; max?: number; label: string; tone?: UiTone; class?: string } = $props();

  const share = $derived(max > 0 ? Math.min(1, Math.max(0, value / max)) : 0);
</script>

<span class={cn("inline-block h-1.5 w-16 overflow-hidden rounded-full bg-hover", klass)} role="img" aria-label="{label}: {Math.round(share * 100)}%" title="{label}: {Math.round(share * 100)}%">
  <span class={cn("block h-full rounded-full transition-[width] duration-200 ease-out", toneClass(tone, "dot"))} style:width="{Math.max(share > 0 ? 6 : 0, share * 100)}%"></span>
</span>
