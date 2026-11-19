<script lang="ts">
  import type { Snippet } from "svelte";
  import { fade, fly } from "svelte/transition";

  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import { cn } from "./cn.ts";
  import { trapFocus } from "./actions.ts";

  /** A sheet that slides in from the right, for looking at one thing next to the list it came from. */
  let {
    open = $bindable(false),
    title,
    subtitle,
    icon,
    width = "md",
    onclose,
    leading,
    actions,
    footer,
    children,
  }: {
    open?: boolean;
    title: string;
    subtitle?: string;
    icon?: string;
    width?: "md" | "lg";
    onclose?: () => void;
    leading?: Snippet;
    actions?: Snippet;
    footer?: Snippet;
    children?: Snippet;
  } = $props();

  const uid = $props.id();
  const widths = { md: "w-[480px]", lg: "w-[640px]" };

  function close() {
    open = false;
    onclose?.();
  }
</script>

<svelte:window onkeydown={(e) => open && e.key === "Escape" && close()} />

{#if open}
  <div class="fixed inset-0 z-40" transition:fade={{ duration: 140 }}>
    <button type="button" class="absolute inset-0 cursor-default bg-black/35" aria-label="Close" tabindex="-1" onclick={close}></button>
  </div>
  <div
    role="dialog"
    aria-modal="true"
    aria-labelledby="{uid}-title"
    use:trapFocus
    transition:fly={{ x: 40, duration: 220, opacity: 0.4 }}
    class={cn("fixed inset-y-0 right-0 z-50 flex max-w-full flex-col border-l border-line bg-elevated shadow-3", widths[width])}
  >
    <header class="flex items-start gap-3 border-b border-line px-5 py-4">
      {#if leading}{@render leading()}{:else if icon}<Icon name={icon} size={18} class="mt-0.5 text-accent" />{/if}
      <div class="min-w-0 flex-1">
        <h2 id="{uid}-title" data-autofocus tabindex="-1" class="truncate text-xl font-semibold outline-none">{title}</h2>
        {#if subtitle}<p class="mt-0.5 truncate text-sm text-muted">{subtitle}</p>{/if}
      </div>
      {@render actions?.()}
      <IconButton label="Close" icon="x" size="sm" onclick={close} />
    </header>
    <div class="min-h-0 flex-1 overflow-y-auto">{@render children?.()}</div>
    {#if footer}<footer class="flex items-center justify-end gap-2 border-t border-line bg-panel-2 px-5 py-3">{@render footer()}</footer>{/if}
  </div>
{/if}
