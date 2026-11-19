<script lang="ts">
  import { cn } from "./cn.ts";
  import { type UiTone, toneClass } from "./tone.ts";

  /** An agent's face: an emoji on a soft tinted tile. Decorative unless a label is given. */
  let {
    emoji = "🤖",
    color = "accent",
    size = "md",
    label,
    class: klass = "",
  }: { emoji?: string; color?: string; size?: "xs" | "sm" | "md" | "lg" | "xl"; label?: string; class?: string } = $props();

  const tone = $derived((["accent", "ok", "warn", "err", "info", "muted"].includes(color) ? color : "accent") as UiTone);
  const sizes = {
    xs: "size-5 rounded-md text-[11px]",
    sm: "size-7 rounded-lg text-[14px]",
    md: "size-9 rounded-xl text-[18px]",
    lg: "size-12 rounded-2xl text-[24px]",
    xl: "size-16 rounded-[20px] text-[32px]",
  };
</script>

<span
  class={cn("inline-grid shrink-0 place-items-center leading-none select-none inset-ring inset-ring-white/20", toneClass(tone, "soft"), sizes[size], klass)}
  role={label ? "img" : undefined}
  aria-label={label}
  aria-hidden={label ? undefined : "true"}
>{emoji}</span>
