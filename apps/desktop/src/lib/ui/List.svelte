<script lang="ts">
  import type { Snippet } from "svelte";

  import { cn } from "./cn.ts";

  let { class: klass = "", children, label }: { class?: string; children: Snippet; label?: string } = $props();
</script>

<ul aria-label={label} class={cn("divide-y divide-line overflow-hidden rounded-2xl border border-line bg-panel shadow-card", klass)}>
  {@render children()}
</ul>
