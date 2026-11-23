// What New run tells a person before they start: where the conversation goes, and what the run may use. Pure, tested in newRun.test.ts.
import type { ModelRow } from "./api.ts";
import { t as tr } from "./i18n/index.ts";

export type Destination =
  | { kind: "demo" }
  | { kind: "local"; provider: string }
  | { kind: "cloud"; provider: string }
  /** An alias or a route: where it ends up is decided by the router, not known here. */
  | { kind: "routed"; name: string }
  | { kind: "none" };

const PROVIDER_NAMES: Record<string, string> = { anthropic: tr("newRun.anthropic"), openai: tr("newRun.openai"), gemini: tr("newRun.gemini"), ollama: tr("newRun.ollama") };

/** `anthropic` becomes "Anthropic"; a provider this does not know keeps its own name. */
export const providerName = (id: string): string => PROVIDER_NAMES[id] ?? id;

/**
 * Where the conversation of a run goes. `model` is what the person chose ("" means the default), `defaultRef` what the default means
 * (a `provider/model` reference, an alias, or nothing), `models` the catalogue that says which are local.
 */
export function destination(model: string, defaultRef: string | null, models: readonly Pick<ModelRow, "reference" | "local">[]): Destination {
  const ref = model || defaultRef;
  if (!ref) return { kind: "demo" };
  if (ref === "demo") return { kind: "demo" };
  if (ref.startsWith("route:")) return { kind: "routed", name: ref.slice(6) };
  const slash = ref.indexOf("/");
  if (slash < 0) return { kind: "routed", name: ref };
  const provider = ref.slice(0, slash);
  const row = models.find((m) => m.reference === ref);
  return row?.local || provider === "ollama" ? { kind: "local", provider } : { kind: "cloud", provider };
}

/** How many of the chosen tools ask first, and how many are granted in all. */
export function grantSummary(tools: readonly { requires_approval: boolean }[]): { granted: number; asks: number } {
  return { granted: tools.length, asks: tools.filter((t) => t.requires_approval).length };
}
