import { t } from "../i18n/index.ts";
// The one place that maps a tone to Tailwind classes. Class names are written out in full so Tailwind can see them.

export type UiTone = "ok" | "warn" | "err" | "info" | "muted" | "accent";
export type ToneKind = "text" | "dot" | "soft" | "border" | "solid" | "callout";

const MAP: Record<UiTone, Record<ToneKind, string>> = {
  ok: {
    text: "text-ok",
    dot: "bg-ok",
    soft: t("ui.tone.bgOk12Text"),
    border: "border-ok/40",
    solid: t("ui.tone.bgOkTextBg"),
    callout: t("ui.tone.borderOk35Bg"),
  },
  warn: {
    text: "text-warn",
    dot: "bg-warn",
    soft: t("ui.tone.bgWarn14Text"),
    border: "border-warn/45",
    solid: t("ui.tone.bgWarnTextBg"),
    callout: t("ui.tone.borderWarn40Bg"),
  },
  err: {
    text: "text-err",
    dot: "bg-err",
    soft: t("ui.tone.bgErr12Text"),
    border: "border-err/40",
    solid: t("ui.tone.bgErrTextBg"),
    callout: t("ui.tone.borderErr35Bg"),
  },
  info: {
    text: "text-info",
    dot: "bg-info",
    soft: t("ui.tone.bgInfo12Text"),
    border: "border-info/40",
    solid: t("ui.tone.bgInfoTextBg"),
    callout: t("ui.tone.borderInfo35Bg"),
  },
  muted: {
    text: "text-muted",
    dot: "bg-faint",
    soft: t("ui.tone.bgHoverTextMuted"),
    border: "border-line",
    solid: t("ui.tone.bgMutedTextBg"),
    callout: t("ui.tone.borderLineBgPanel"),
  },
  accent: {
    text: "text-accent",
    dot: "bg-accent",
    soft: t("ui.tone.bgAccentSoftText"),
    border: "border-accent/40",
    solid: t("ui.tone.bgAccentTextAccent"),
    callout: t("ui.tone.borderAccent35Bg"),
  },
};

export const TONES = Object.keys(MAP) as UiTone[];
export const TONE_KINDS: ToneKind[] = ["text", "dot", "soft", "border", "solid", "callout"];

/** The classes for a tone: text colour, a dot, a soft tinted badge, a border, a solid fill or a callout box. */
export function toneClass(tone: UiTone, kind: ToneKind = "text"): string {
  return MAP[tone][kind];
}
