<script lang="ts">
  import type { Snippet } from "svelte";

  import Icon from "./Icon.svelte";
  import { cn } from "./cn.ts";

  let { icon = "sparkles", title, text, actions, class: klass = "" }: { icon?: string; title: string; text?: string; actions?: Snippet; class?: string } = $props();
</script>

<div class={cn("mx-auto flex max-w-sm flex-col items-center px-6 py-12 text-center", klass)}>
  <div class="brand-gradient grid size-14 place-items-center rounded-[18px] text-white shadow-[0_16px_36px_-14px_var(--accent)]">
    <Icon name={icon} size={26} stroke={1.7} />
  </div>
  <h3 class="mt-5 text-2xl font-semibold">{title}</h3>
  {#if text}<p class="mt-1.5 text-base text-muted">{text}</p>{/if}
  {#if actions}<div class="mt-5 flex flex-wrap justify-center gap-2">{@render actions()}</div>{/if}
</div>
