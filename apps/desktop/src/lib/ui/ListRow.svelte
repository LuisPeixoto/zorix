<script lang="ts">
  import type { Snippet } from "svelte";

  import { cn } from "./cn.ts";

  let { active = false, onclick, class: klass = "", children }: { active?: boolean; onclick?: () => void; class?: string; children: Snippet } = $props();
</script>

<li class={cn(active && "bg-accent-soft")}>
  {#if onclick}
    <button type="button" {onclick} class={cn("flex w-full items-center gap-3 px-4 py-3 text-left transition duration-150 hover:bg-hover", klass)}>{@render children()}</button>
  {:else}
    <div class={cn("flex items-center gap-3 px-4 py-3", klass)}>{@render children()}</div>
  {/if}
</li>
