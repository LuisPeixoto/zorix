<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLButtonAttributes } from "svelte/elements";

  import Icon from "./Icon.svelte";
  import Spinner from "./Spinner.svelte";
  import { cn } from "./cn.ts";

  type Props = HTMLButtonAttributes & {
    variant?: "primary" | "secondary" | "ghost" | "danger" | "subtle";
    size?: "sm" | "md" | "lg";
    icon?: string;
    iconRight?: string;
    loading?: boolean;
    children?: Snippet;
  };

  let { variant = "secondary", size = "md", icon, iconRight, loading = false, disabled = false, type = "button", class: klass = "", children, ...rest }: Props = $props();

  const variants = {
    primary:
      "surface-primary hover:brightness-105",
    secondary: "border border-line bg-panel text-fg shadow-1 raised hover:border-line-strong hover:bg-hover",
    ghost: "text-muted hover:bg-hover hover:text-fg",
    danger: "border border-err/40 bg-err/8 text-err hover:bg-err/15",
    subtle: "bg-accent-soft text-accent hover:bg-select",
  };
  const sizes = { sm: "h-7 px-2.5 text-sm", md: "h-8 px-3.5 text-base", lg: "h-10 px-5 text-md" };
  const iconSize = { sm: 14, md: 15, lg: 17 };
</script>

<button
  {type}
  disabled={disabled || loading}
  aria-busy={loading || undefined}
  class={cn(
    "relative inline-flex shrink-0 items-center justify-center gap-1.5 rounded-lg font-medium whitespace-nowrap transition duration-150 ease-out active:scale-[0.98]",
    variants[variant],
    sizes[size],
    klass,
  )}
  {...rest}
>
  <span class={cn("inline-flex flex-1 items-center justify-center gap-1.5", loading && "invisible")}>
    {#if icon}<Icon name={icon} size={iconSize[size]} />{/if}
    {@render children?.()}
    {#if iconRight}<Icon name={iconRight} size={iconSize[size]} />{/if}
  </span>
  {#if loading}<span class="absolute inset-0 grid place-items-center"><Spinner size={iconSize[size]} /></span>{/if}
</button>
