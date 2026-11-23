<script lang="ts">
  import { t } from "../i18n/index.ts";
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
      t("ui.button.surfacePrimaryHoverBrightness"),
    secondary: t("ui.button.borderBorderLineBg"),
    ghost: t("ui.button.textMutedHoverBg"),
    danger: t("ui.button.borderBorderErr40"),
    subtle: t("ui.button.bgAccentSoftText"),
  };
  const sizes = { sm: t("ui.button.h7Px2"), md: t("ui.button.h8Px3"), lg: t("ui.button.h10Px5") };
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
