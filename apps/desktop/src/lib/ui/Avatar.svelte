<script lang="ts">
  import { cn } from "./cn.ts";
  import { type UiTone, toneClass } from "./tone.ts";

  /** An agent's face: an emoji on a soft tinted square. Decorative unless a label is given. People are round initials, never this. */
  let {
    emoji = "🤖",
    color = "accent",
    size = "md",
    label,
    class: klass = "",
  }: { emoji?: string; color?: string; size?: "xs" | "sm" | "md" | "lg" | "xl"; label?: string; class?: string } = $props();

  const tone = $derived((["accent", "ok", "warn", "err", "info", "muted"].includes(color) ? color : "accent") as UiTone);
  // The box is a scale step and the radius a token; the emoji is half the box, as in the design system's 32 px avatar with a 16 px face.
  const sizes = {
    xs: { box: "size-5 rounded-md", px: 20 },
    sm: { box: "size-7 rounded-lg", px: 28 },
    md: { box: "size-9 rounded-xl", px: 36 },
    lg: { box: "size-12 rounded-2xl", px: 48 },
    xl: { box: "size-16 rounded-2xl", px: 64 },
  };
</script>

<span
  class={cn("inline-grid shrink-0 place-items-center border border-current/25 leading-none select-none", toneClass(tone, "soft"), sizes[size].box, klass)}
  style:font-size="{sizes[size].px / 2}px"
  role={label ? "img" : undefined}
  aria-label={label}
  aria-hidden={label ? undefined : "true"}
>{emoji}</span>
