<script lang="ts">
  import type { Snippet } from "svelte";

  import Switch from "./Switch.svelte";
  import { cn } from "./cn.ts";

  /** A setting with a name, a sentence about it, and a switch. */
  let {
    checked = $bindable(false),
    label,
    description,
    disabled = false,
    onchange,
    meta,
    class: klass = "",
  }: {
    checked?: boolean;
    label: string;
    description?: string;
    disabled?: boolean;
    onchange?: (checked: boolean) => void;
    meta?: Snippet;
    class?: string;
  } = $props();

  const uid = $props.id();
</script>

<div class={cn("flex items-start gap-4 px-4 py-3", klass)}>
  <div class="min-w-0 flex-1">
    <p id="{uid}-label" class="flex items-center gap-2 text-md font-medium">{label}{@render meta?.()}</p>
    {#if description}<p id="{uid}-desc" class="mt-0.5 text-sm text-muted">{description}</p>{/if}
  </div>
  <Switch bind:checked {disabled} aria-labelledby="{uid}-label" aria-describedby={description ? `${uid}-desc` : undefined} onchange={() => onchange?.(checked)} />
</div>
