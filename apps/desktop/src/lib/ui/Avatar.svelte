<script lang="ts">
  import { t } from "../i18n/index.ts";
  import Icon from "./Icon.svelte";
  import { cn } from "./cn.ts";
  import { type UiTone, toneClass } from "./tone.ts";

  /**
   * An agent's face: an emoji on a soft tinted square. Decorative unless a label is given. A person is round, so a human is never
   * mistaken for an agent: with `human` it shows `initials`, or a person's icon when there are none.
   */
  let {
    emoji = "🤖",
    color = "accent",
    size = "md",
    label,
    human = false,
    initials,
    status,
    class: klass = "",
  }: {
    emoji?: string;
    color?: string;
    size?: "xs" | "sm" | "md" | "lg" | "xl";
    label?: string;
    human?: boolean;
    initials?: string;
    /** A dot on the corner: `ok` working, `warn` waiting for a person, `err` stopped. None when idle. */
    status?: "ok" | "warn" | "err" | null;
    class?: string;
  } = $props();

  const tone = $derived((human ? "muted" : ["accent", "ok", "warn", "err", "info", "muted"].includes(color) ? color : "accent") as UiTone);
  // The box is a scale step and the radius a token; the emoji is half the box, as in the design system's 32 px avatar with a 16 px face.
  const sizes = {
    xs: { box: t("ui.avatar.size5RoundedMd"), px: 20 },
    sm: { box: t("ui.avatar.size7RoundedLg"), px: 28 },
    md: { box: t("ui.avatar.size9RoundedXl"), px: 36 },
    lg: { box: t("ui.avatar.size12Rounded2xl"), px: 48 },
    xl: { box: t("ui.avatar.size16Rounded2xl"), px: 64 },
  };
</script>

<span
  class={cn("relative inline-grid shrink-0 place-items-center border border-current/25 leading-none select-none", toneClass(tone, "soft"), sizes[size].box, human && "rounded-full font-semibold", klass)}
  style:font-size="{human ? sizes[size].px * 0.36 : sizes[size].px / 2}px"
  role={label ? "img" : undefined}
  aria-label={label}
  aria-hidden={label ? undefined : "true"}
>{#if human}{#if initials}{initials}{:else}<Icon name="user" size={Math.round(sizes[size].px * 0.5)} />{/if}{:else}{emoji}{/if}{#if status}<span class="absolute -right-0.5 -bottom-0.5 size-2.5 rounded-full border-2 border-panel {status === 'ok' ? 'bg-ok' : status === 'warn' ? 'bg-warn' : 'bg-err'}" title={status === "ok" ? t("common.working") : status === "warn" ? t("common.waitingForYou") : t("common.stopped")}></span>{/if}</span>
