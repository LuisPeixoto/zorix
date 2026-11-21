<script lang="ts">
  import type { Snippet } from "svelte";

  import { cn } from "./cn.ts";

  let { title, subtitle, actions, glow = false, class: klass = "" }: { title: string; subtitle?: string; actions?: Snippet; glow?: boolean; class?: string } = $props();
</script>

<header class={cn("relative mb-6 flex flex-wrap items-end gap-4 border-b border-line-soft pb-4", klass)}>
  {#if glow}<span class="glow -top-20 -left-16 size-56" aria-hidden="true"></span>{/if}
  <div class="relative min-w-0 flex-1">
    <h1 class="text-3xl font-semibold tracking-tight">{title}</h1>
    {#if subtitle}<p class="mt-1 text-base text-muted">{subtitle}</p>{/if}
  </div>
  {#if actions}<div class="relative flex items-center gap-2">{@render actions()}</div>{/if}
</header>
