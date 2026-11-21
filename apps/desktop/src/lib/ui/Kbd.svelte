<script lang="ts">
  import type { Snippet } from "svelte";
  let { children }: { children: Snippet } = $props();
</script>

<kbd class="mono inline-flex h-5 min-w-5 items-center justify-center rounded-sm border border-b-2 border-line bg-panel px-1.5 text-xs font-medium text-muted">{@render children()}</kbd>
