<script lang="ts">
  import { cn } from "./cn.ts";

  /** Facts about something, as label and value pairs. Values are plain text; `mono` is for ids, paths and hashes. */
  let {
    items,
    class: klass = "",
  }: { items: { key: string; value: string | number | null | undefined; mono?: boolean }[]; class?: string } = $props();
</script>

<dl class={cn("grid grid-cols-[max-content_1fr] gap-x-4 gap-y-2 text-base", klass)}>
  {#each items.filter((i) => i.value !== null && i.value !== undefined && i.value !== "") as item (item.key)}
    <dt class="text-muted">{item.key}</dt>
    <dd class={cn("selectable min-w-0 break-words", item.mono && "mono text-sm")}>{item.value}</dd>
  {/each}
</dl>
