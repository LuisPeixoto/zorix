<script lang="ts">
  import type { Snippet } from "svelte";
  import { fade, scale } from "svelte/transition";

  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import { cn } from "./cn.ts";
  import { trapFocus } from "./actions.ts";

  let {
    open = $bindable(false),
    title,
    icon,
    size = "md",
    onclose,
    footer,
    children,
  }: {
    open?: boolean;
    title: string;
    icon?: string;
    size?: "sm" | "md" | "lg";
    onclose?: () => void;
    footer?: Snippet;
    children?: Snippet;
  } = $props();

  const uid = $props.id();
  const widths = { sm: "max-w-sm", md: "max-w-lg", lg: "max-w-2xl" };

  function close() {
    open = false;
    onclose?.();
  }
</script>

<svelte:window onkeydown={(e) => open && e.key === "Escape" && close()} />

{#if open}
  <div transition:fade={{ duration: 120 }} class="fixed inset-0 z-50 grid place-items-center bg-black/45 p-8 backdrop-blur-[2px]">
    <button type="button" class="absolute inset-0 cursor-default" aria-label="Close" tabindex="-1" onclick={close}></button>
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="{uid}-title"
      transition:scale={{ start: 0.97, duration: 160 }}
      use:trapFocus
      class={cn("relative flex max-h-full w-full flex-col overflow-hidden rounded-2xl border border-line bg-elevated shadow-3", widths[size])}
    >
      <header class="flex items-center gap-2.5 border-b border-line px-5 py-3.5">
        {#if icon}<Icon name={icon} size={17} class="text-accent" />{/if}
        <h2 id="{uid}-title" data-autofocus tabindex="-1" class="min-w-0 flex-1 truncate text-xl font-semibold outline-none">{title}</h2>
        <IconButton label="Close" icon="x" size="sm" onclick={close} />
      </header>
      <div class="min-h-0 flex-1 overflow-y-auto px-5 py-4">{@render children?.()}</div>
      {#if footer}<footer class="flex items-center justify-end gap-2 border-t border-line bg-panel-2 px-5 py-3">{@render footer()}</footer>{/if}
    </div>
  </div>
{/if}
