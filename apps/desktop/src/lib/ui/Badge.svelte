<script lang="ts">
  import type { Snippet } from "svelte";

  import Dot from "./Dot.svelte";
  import { cn } from "./cn.ts";
  import { type UiTone, toneClass } from "./tone.ts";

  let {
    tone = "muted",
    variant = "soft",
    dot = false,
    live = false,
    class: klass = "",
    children,
  }: { tone?: UiTone; variant?: "soft" | "outline" | "solid"; dot?: boolean; live?: boolean; class?: string; children: Snippet } = $props();
</script>

<span
  class={cn(
    "inline-flex h-6 items-center gap-1.5 rounded-full px-2.5 text-xs font-medium whitespace-nowrap",
    variant === "soft" && toneClass(tone, "soft"),
    variant === "outline" && ["border bg-panel", toneClass(tone, "border"), toneClass(tone, "text")],
    variant === "solid" && toneClass(tone, "solid"),
    klass,
  )}
>
  {#if dot || live}<Dot {tone} {live} class="size-1.5" />{/if}
  {@render children()}
</span>
