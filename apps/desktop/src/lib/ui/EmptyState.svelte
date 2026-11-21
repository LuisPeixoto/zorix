<script lang="ts">
  import type { Snippet } from "svelte";

  import BrandMark from "./BrandMark.svelte";
  import { cn } from "./cn.ts";

  /** What a screen shows before it has anything: an icon tile, one sentence and the first action. */
  let { icon = "sparkles", title, text, actions, class: klass = "" }: { icon?: string; title: string; text?: string; actions?: Snippet; class?: string } = $props();
</script>

<div class={cn("relative mx-auto flex w-full max-w-xl flex-col items-center overflow-hidden rounded-2xl border border-dashed border-line-strong bg-panel px-6 py-10 text-center", klass)}>
  <span class="glow -top-16 left-1/2 h-36 w-72 -translate-x-1/2" aria-hidden="true"></span>
  <BrandMark size="lg" {icon} />
  <h3 class="relative mt-3.5 text-xl font-semibold">{title}</h3>
  {#if text}<p class="relative mt-1.5 max-w-[40ch] text-base leading-relaxed text-muted">{text}</p>{/if}
  {#if actions}<div class="relative mt-4 flex flex-wrap justify-center gap-2">{@render actions()}</div>{/if}
</div>
