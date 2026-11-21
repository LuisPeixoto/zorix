<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLInputAttributes } from "svelte/elements";

  import Icon from "./Icon.svelte";
  import { cn } from "./cn.ts";

  type Props = Omit<HTMLInputAttributes, "type"> & { checked?: boolean; description?: string; children?: Snippet };
  let { checked = $bindable(false), description, disabled = false, class: klass = "", children, ...rest }: Props = $props();
</script>

<label class={cn("flex items-start gap-2.5", disabled ? "cursor-not-allowed opacity-50" : "cursor-pointer", klass)}>
  <input type="checkbox" bind:checked {disabled} class="peer sr-only" {...rest} />
  <span
    class="mt-px grid size-4 shrink-0 place-items-center rounded-sm border border-line-strong bg-panel text-accent-fg transition duration-150 peer-checked:border-accent peer-checked:bg-accent peer-focus-visible:ring-3 peer-focus-visible:ring-accent-soft peer-checked:[&>svg]:scale-100 peer-checked:[&>svg]:opacity-100"
  >
    <Icon name="check" size={12} stroke={3} class="scale-50 opacity-0 transition duration-150" />
  </span>
  <span class="min-w-0">
    <span class="text-base text-fg">{@render children?.()}</span>
    {#if description}<span class="block text-xs text-muted">{description}</span>{/if}
  </span>
</label>
