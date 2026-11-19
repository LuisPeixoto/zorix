<script lang="ts">
  import type { Snippet } from "svelte";

  import Icon from "./Icon.svelte";
  import { cn } from "./cn.ts";

  let {
    title,
    icon,
    padding = "md",
    interactive = false,
    actions,
    footer,
    class: klass = "",
    children,
  }: {
    title?: string;
    icon?: string;
    padding?: "none" | "sm" | "md";
    interactive?: boolean;
    actions?: Snippet;
    footer?: Snippet;
    class?: string;
    children?: Snippet;
  } = $props();

  const pad = { none: "", sm: "p-3", md: "p-4" };
</script>

<section
  class={cn(
    "raised overflow-hidden rounded-2xl border border-line bg-panel",
    interactive && "transition duration-150 ease-out hover:-translate-y-px hover:border-line-strong hover:shadow-3",
    klass,
  )}
>
  {#if title || actions}
    <header class="flex items-center gap-2 border-b border-line-soft px-4 py-3">
      {#if icon}<Icon name={icon} size={15} class="text-accent" />{/if}
      <h3 class="min-w-0 flex-1 truncate text-md font-semibold">{title}</h3>
      {@render actions?.()}
    </header>
  {/if}
  <div class={pad[padding]}>{@render children?.()}</div>
  {#if footer}<footer class="border-t border-line bg-panel-2 px-4 py-2.5">{@render footer()}</footer>{/if}
</section>
