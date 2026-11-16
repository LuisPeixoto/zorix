<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLSelectAttributes } from "svelte/elements";

  import Icon from "./Icon.svelte";
  import { cn } from "./cn.ts";

  type Props = HTMLSelectAttributes & { value?: string; invalid?: boolean; children: Snippet };
  let { value = $bindable(""), invalid = false, class: klass = "", children, ...rest }: Props = $props();
</script>

<div class={cn("relative flex items-center", klass)}>
  <select
    bind:value
    aria-invalid={invalid || undefined}
    class={cn(
      "h-8 w-full cursor-pointer appearance-none rounded-lg border bg-panel-2 pr-8 pl-2.5 text-base text-fg outline-none transition duration-150 hover:border-line-strong focus:bg-panel focus:ring-3",
      invalid ? "border-err focus:border-err focus:ring-err/20" : "border-line focus:border-accent focus:ring-accent-soft",
    )}
    {...rest}
  >
    {@render children()}
  </select>
  <Icon name="chevron-down" size={15} class="pointer-events-none absolute right-2.5 text-faint" />
</div>
