<script lang="ts">
  import { t } from "../../i18n/index.ts";
  import type { Snippet } from "svelte";

  import { Icon, cn } from "../../ui/index.ts";

  /** One entry of the sidebar. Active is a tint with accent text, never a bar. A count in warn means something needs a person. */
  let {
    icon,
    active = false,
    count,
    attention = false,
    onclick,
    children,
  }: { icon: string; active?: boolean; count?: number; attention?: boolean; onclick: () => void; children: Snippet } = $props();
</script>

<button
  type="button"
  {onclick}
  aria-current={active ? "page" : undefined}
  class={cn(
    "group relative flex h-7.5 w-full items-center gap-2.5 rounded-lg px-2.5 text-base font-medium transition duration-150",
    active ? "bg-accent-soft text-accent" : "text-muted hover:bg-hover hover:text-fg",
  )}
>
  <Icon name={icon} size={15} class={active ? "text-accent" : "text-faint group-hover:text-muted"} />
  <span class="min-w-0 flex-1 truncate text-left">{@render children()}</span>
  {#if count !== undefined && count > 0}
    <span class={cn("tnum rounded-full px-1.5 text-2xs font-semibold", attention ? "bg-warn text-bg" : "bg-hover text-muted")}>
      {count}<span class="sr-only">{attention ? t("shell.navItem.needAttention") : ""}</span>
    </span>
  {:else if attention}
    <span class="size-1.5 rounded-full bg-warn" role="img" aria-label={t("shell.navItem.needsAttention")}></span>
  {/if}
</button>
