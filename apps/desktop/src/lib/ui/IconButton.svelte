<script lang="ts">
  import { t } from "../i18n/index.ts";
  import type { HTMLButtonAttributes } from "svelte/elements";

  import Icon from "./Icon.svelte";
  import { cn } from "./cn.ts";
  import { tooltip } from "./actions.ts";

  type Props = HTMLButtonAttributes & {
    /** What it does, for screen readers and the tooltip. Required. */
    label: string;
    icon: string;
    variant?: "ghost" | "secondary" | "danger";
    size?: "sm" | "md";
  };

  let { label, icon, variant = "ghost", size = "md", type = "button", class: klass = "", ...rest }: Props = $props();

  const variants = {
    ghost: t("ui.iconButton.textMutedHoverBg"),
    secondary: t("ui.iconButton.borderBorderLineBg"),
    danger: t("ui.iconButton.textErrHoverBg"),
  };
</script>

<button
  {type}
  aria-label={label}
  use:tooltip={label}
  class={cn("grid shrink-0 place-items-center rounded-lg transition duration-150 ease-out active:scale-95", size === "sm" ? "size-7" : "size-8", variants[variant], klass)}
  {...rest}
>
  <Icon name={icon} size={size === "sm" ? 14 : 16} />
</button>
