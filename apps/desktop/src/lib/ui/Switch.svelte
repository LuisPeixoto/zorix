<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLInputAttributes } from "svelte/elements";

  import { cn } from "./cn.ts";

  type Props = Omit<HTMLInputAttributes, "type"> & { checked?: boolean; children?: Snippet };
  let { checked = $bindable(false), disabled = false, class: klass = "", children, ...rest }: Props = $props();
</script>

<label class={cn("inline-flex items-center gap-2.5", disabled ? "cursor-not-allowed opacity-50" : "cursor-pointer", klass)}>
  <input type="checkbox" role="switch" bind:checked {disabled} class="peer sr-only" {...rest} />
  <span
    class="relative h-4.5 w-8 shrink-0 rounded-full bg-line-strong transition duration-200 peer-checked:bg-accent peer-focus-visible:ring-3 peer-focus-visible:ring-accent-soft peer-checked:[&>span]:translate-x-3.5"
  >
    <span class="absolute top-0.5 left-0.5 size-3.5 rounded-full bg-white shadow-1 transition duration-200 ease-spring"></span>
  </span>
  {#if children}<span class="text-base text-fg">{@render children()}</span>{/if}
</label>
