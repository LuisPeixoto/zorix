<script lang="ts">
  import type { Snippet } from "svelte";

  import { Icon, cn } from "../../ui/index.ts";

  let { icon, active = false, onclick, children }: { icon: string; active?: boolean; onclick: () => void; children: Snippet } = $props();
</script>

<button
  type="button"
  {onclick}
  aria-current={active ? "page" : undefined}
  class={cn(
    "flex h-8 w-full items-center gap-2.5 rounded-lg px-2.5 text-md transition duration-150",
    active ? "bg-accent-soft font-medium text-accent" : "text-fg/85 hover:bg-hover",
  )}
>
  <Icon name={icon} size={15} />
  {@render children()}
</button>
