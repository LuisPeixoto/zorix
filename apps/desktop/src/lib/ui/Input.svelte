<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLInputAttributes } from "svelte/elements";

  import Icon from "./Icon.svelte";
  import { cn } from "./cn.ts";

  type Props = Omit<HTMLInputAttributes, "size"> & {
    value?: string;
    icon?: string;
    invalid?: boolean;
    mono?: boolean;
    size?: "sm" | "md";
    trailing?: Snippet;
  };

  let { value = $bindable(""), icon, invalid = false, mono = false, size = "md", trailing, class: klass = "", ...rest }: Props = $props();
</script>

<div class={cn("relative flex items-center", klass)}>
  {#if icon}<Icon name={icon} size={15} class="pointer-events-none absolute left-2.5 text-faint" />{/if}
  <input
    bind:value
    aria-invalid={invalid || undefined}
    class={cn(
      "w-full rounded-lg border bg-panel-2 text-fg outline-none transition duration-150 placeholder:text-faint hover:border-line-strong focus:bg-panel focus:ring-3",
      size === "sm" ? "h-7 text-sm" : "h-8 text-base",
      icon ? "pl-8" : "pl-2.5",
      trailing ? "pr-9" : "pr-2.5",
      mono && "font-mono text-sm",
      invalid ? "border-err focus:border-err focus:ring-err/20" : "border-line focus:border-accent focus:ring-accent-soft",
    )}
    {...rest}
  />
  {#if trailing}<span class="absolute right-1.5 flex items-center">{@render trailing()}</span>{/if}
</div>
