// The one place that maps a tone to Tailwind classes. Class names are written out in full so Tailwind can see them.

export type UiTone = "ok" | "warn" | "err" | "info" | "muted" | "accent";
export type ToneKind = "text" | "dot" | "soft" | "border" | "solid" | "callout";

const MAP: Record<UiTone, Record<ToneKind, string>> = {
  ok: {
    text: "text-ok",
    dot: "bg-ok",
    soft: "bg-ok/12 text-ok",
    border: "border-ok/40",
    solid: "bg-ok text-bg",
    callout: "border-ok/35 bg-ok/8",
  },
  warn: {
    text: "text-warn",
    dot: "bg-warn",
    soft: "bg-warn/14 text-warn",
    border: "border-warn/45",
    solid: "bg-warn text-bg",
    callout: "border-warn/40 bg-warn/10",
  },
  err: {
    text: "text-err",
    dot: "bg-err",
    soft: "bg-err/12 text-err",
    border: "border-err/40",
    solid: "bg-err text-bg",
    callout: "border-err/35 bg-err/8",
  },
  info: {
    text: "text-info",
    dot: "bg-info",
    soft: "bg-info/12 text-info",
    border: "border-info/40",
    solid: "bg-info text-bg",
    callout: "border-info/35 bg-info/8",
  },
  muted: {
    text: "text-muted",
    dot: "bg-faint",
    soft: "bg-hover text-muted",
    border: "border-line",
    solid: "bg-muted text-bg",
    callout: "border-line bg-panel-2",
  },
  accent: {
    text: "text-accent",
    dot: "bg-accent",
    soft: "bg-accent-soft text-accent",
    border: "border-accent/40",
    solid: "bg-accent text-accent-fg",
    callout: "border-accent/35 bg-accent-soft",
  },
};

export const TONES = Object.keys(MAP) as UiTone[];
export const TONE_KINDS: ToneKind[] = ["text", "dot", "soft", "border", "solid", "callout"];

/** The classes for a tone: text colour, a dot, a soft tinted badge, a border, a solid fill or a callout box. */
export function toneClass(tone: UiTone, kind: ToneKind = "text"): string {
  return MAP[tone][kind];
}
