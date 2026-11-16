<script lang="ts">
  import type { Snippet } from "svelte";
  let { children }: { children: Snippet } = $props();
</script>

<kbd class="mono inline-flex h-5 min-w-5 items-center justify-center rounded-md border border-line-strong bg-panel-2 px-1.5 text-2xs text-muted shadow-[0_1px_0_var(--line-strong)]">{@render children()}</kbd>
