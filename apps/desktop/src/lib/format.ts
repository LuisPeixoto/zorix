// Turning engine data into words and short strings. Pure functions, tested in format.test.ts.
import type { ExecEvent, ExecutionState } from "./api.ts";
import { t } from "./i18n/index.ts";

export type Tone = "ok" | "warn" | "err" | "info" | "muted";

const LABELS: Record<ExecutionState, string> = {
  created: t("format.created"),
  queued: t("format.queued"),
  running: t("common.running"),
  waiting_for_model: t("format.thinking"),
  waiting_for_tool: t("format.usingATool"),
  waiting_for_approval: t("format.needsYourApproval"),
  waiting_for_children: t("format.waitingForSubRuns"),
  paused: t("common.paused"),
  completed: t("format.completed"),
  failed: t("common.failed"),
  cancelled: t("common.cancelled"),
};

export const stateLabel = (state: ExecutionState): string => LABELS[state] ?? state;

export function stateTone(state: ExecutionState): Tone {
  switch (state) {
    case "completed":
      return "ok";
    case "failed":
      return "err";
    case "waiting_for_approval":
      return "warn";
    case "running":
    case "waiting_for_children":
    case "waiting_for_model":
    case "waiting_for_tool":
    case "queued":
      return "info";
    default:
      return "muted";
  }
}

/** True while the engine may still change the execution without anyone asking. */
export const isLive = (state: ExecutionState): boolean =>
  ["queued", "running", "waiting_for_model", "waiting_for_tool"].includes(state);

export const isFinished = (state: ExecutionState): boolean => ["completed", "failed", "cancelled"].includes(state);

export const shortId = (id: string): string => id.slice(-8);

export function duration(ms: number | null | undefined): string {
  if (ms == null) return "–";
  if (ms < 1000) return t("format.ms", { value: Math.round(ms) });
  const seconds = ms / 1000;
  if (seconds < 60) return t("format.s", { value: seconds < 10 ? seconds.toFixed(1) : Math.round(seconds) });
  const minutes = Math.floor(seconds / 60);
  return t("format.minS", { m: minutes, s: Math.round(seconds % 60) });
}

export function ago(at: number, now: number = Date.now()): string {
  const seconds = Math.max(0, Math.round((now - at) / 1000));
  if (seconds < 5) return t("format.justNow");
  if (seconds < 60) return t("format.sAgo", { seconds });
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return t("format.minAgo", { minutes });
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return t("format.hAgo", { hours });
  return t("format.dAgo", { value: Math.floor(hours / 24) });
}

export function shorten(text: string, max: number): string {
  const flat = text.replace(/\s+/g, " ").trim();
  return flat.length <= max ? flat : `${flat.slice(0, Math.max(0, max - 1))}…`;
}

export const tokens = (usage: { input_tokens: number; output_tokens: number }): number =>
  usage.input_tokens + usage.output_tokens;

export const json = (value: unknown): string => JSON.stringify(value, null, 2) ?? "";

export interface EventView {
  title: string;
  detail: string;
  tone: Tone;
}

/** What a tool's server asked of the person during the call, and what it was told. */
function asked(payload: Record<string, unknown>): string {
  const requests = Array.isArray(payload.server_requests) ? (payload.server_requests as { server: string; kind: string; outcome: string }[]) : [];
  return requests.map((r) => t(r.kind === "model" ? "format.serverAskedModel" : "format.serverAskedQuestion", { server: r.server, outcome: r.outcome })).join(" · ");
}

/** A person-friendly line for each kind of event. */
export function describeEvent(event: ExecEvent): EventView {
  const p = event.payload;
  const tool = String(p.tool ?? "");
  switch (event.kind) {
    case "execution.created":
      return { title: t("format.created"), detail: `model ${p.model}`, tone: "muted" };
    case "execution.queued":
      return { title: t("format.queued"), detail: "", tone: "muted" };
    case "execution.started":
      return { title: t("format.started"), detail: "", tone: "info" };
    case "execution.resumed":
      return { title: t("format.resumed"), detail: "", tone: "info" };
    case "execution.paused":
      return { title: t("common.paused"), detail: "", tone: "muted" };
    case "execution.recovered":
      return { title: t("format.recoveredAfterARestart"), detail: "", tone: "info" };
    case "execution.model.requested":
      return { title: t("format.askedTheModel"), detail: `step ${p.iteration}`, tone: "info" };
    case "execution.model.responded": {
      const calls = Number(p.tool_calls ?? 0);
      const what = calls ? t("format.wantsToolCalls", { count: calls }) : t("format.finalAnswer");
      const who = p.provider ? ` · ${p.provider}/${p.model}` : "";
      return { title: t("format.modelAnswered"), detail: what + who, tone: "info" };
    }
    case "execution.model.retrying":
      return { title: t("format.modelFailedTryingAgain"), detail: `${p.error} (in ${p.delay_ms} ms)`, tone: "warn" };
    case "execution.model.fallback":
      return { title: t("format.switchedModel"), detail: `${p.from} → ${p.to} (${p.reason})`, tone: "warn" };
    case "execution.context.summarised":
      return {
        title: t("format.summarisedEarlierTurns"),
        detail: t("format.summarisedDetail", { messages: Number(p.folded_messages ?? 0), chars: Number(p.summary_chars ?? 0) }),
        tone: "info",
      };
    case "execution.tool.requested":
      return { title: t("format.toolRequested", { tool }), detail: shorten(JSON.stringify(p.arguments ?? {}), 140), tone: "info" };
    case "execution.tool.started":
      return { title: t("format.running2", { tool }), detail: Number(p.attempt) > 1 ? `attempt ${p.attempt}` : "", tone: "info" };
    case "execution.tool.completed":
      return { title: t("format.finished", { tool }), detail: [duration(Number(p.duration_ms ?? 0)), asked(p)].filter(Boolean).join(" · "), tone: "ok" };
    case "execution.tool.failed":
      return { title: `${tool} failed`, detail: [shorten(String(p.error ?? p.reason ?? ""), 160), asked(p)].filter(Boolean).join(" · "), tone: "err" };
    case "execution.tool.denied":
      return { title: t("format.wasRefused", { tool }), detail: shorten(`${p.reason ?? ""} (by ${p.by})`, 160), tone: "warn" };
    case "execution.waiting_children": {
      const n = Array.isArray(p.children) ? p.children.length : Number(p.tasks ?? 0);
      return { title: t("format.askedOtherAgents"), detail: t("format.tasksAtTheSame", { count: n }), tone: "info" };
    }
    case "execution.child.started":
      return { title: `${p.agent ?? t("format.anAgent")} started`, detail: shorten(String(p.task ?? ""), 140), tone: "info" };
    case "execution.child.settled": {
      const state = String(p.state ?? "");
      return {
        title: `${p.agent ?? t("format.anAgent")} ${state}`,
        detail: `${p.steps ?? 0} step${Number(p.steps) === 1 ? "" : "s"}`,
        tone: state === "completed" ? "ok" : state === "failed" ? "err" : "muted",
      };
    }
    case "execution.waiting_approval":
      return { title: t("format.waitingForApproval", { tool }), detail: String(p.reason ?? ""), tone: "warn" };
    case "execution.approval.granted":
      return { title: t("format.approved"), detail: `by ${p.decided_by}${p.note ? ` – ${p.note}` : ""}`, tone: "ok" };
    case "execution.approval.denied":
      return { title: t("format.refused"), detail: `by ${p.decided_by}${p.note ? ` – ${p.note}` : ""}`, tone: "warn" };
    case "execution.result.rejected":
      return { title: t("format.answerRejected"), detail: String(p.reason ?? ""), tone: "warn" };
    case "execution.completed":
      return { title: t("format.completed"), detail: `${p.iterations} step${Number(p.iterations) === 1 ? "" : "s"}`, tone: "ok" };
    case "execution.failed":
      return { title: t("common.failed"), detail: String(p.message ?? ""), tone: "err" };
    case "execution.cancelled":
      return { title: t("common.cancelled"), detail: String(p.reason ?? ""), tone: "muted" };
    default:
      return { title: event.kind.replace("execution.", ""), detail: "", tone: "muted" };
  }
}

/** Events worth a row in the timeline. Bookkeeping is hidden to keep the story readable. */
export const isNoise = (event: ExecEvent): boolean => ["execution.progress", "execution.checkpoint"].includes(event.kind);

/** The names of the models that served a run, for display: `anthropic/claude-opus-5-5, ollama/qwen3:8b`. */
export function servedBy(models: string[]): string {
  return models.length ? models.join(", ") : "";
}

/** What a provider's key means to a person. */
export function keyLabel(key: string): string {
  switch (key) {
    case "present":
      return t("format.keyStored");
    case "missing":
      return t("format.noKeyYet");
    case "not_needed":
      return t("format.noKeyNeeded");
    default:
      return t("format.keychainUnavailable");
  }
}

/** The tone of a provider's key state, for its badge. */
export function keyTone(key: string): Tone {
  return key === "present" ? "ok" : key === "missing" ? "warn" : key === "unavailable" ? "err" : "muted";
}

/** An icon for each kind of event, so the timeline can be scanned. */
export function eventIcon(kind: string): string {
  if (kind.includes("waiting_children") || kind.includes(".child.")) return "branch";
  if (kind.includes("fallback")) return "route";
  if (kind.includes(".context.")) return "layers";
  if (kind.includes("approval") || kind.includes("waiting_approval")) return "shield";
  if (kind.includes(".tool.")) return "wrench";
  if (kind.includes(".model.")) return "cpu";
  if (kind.endsWith(".completed")) return "check-circle";
  if (kind.endsWith(".failed")) return "x-circle";
  if (kind.endsWith(".cancelled")) return "stop";
  if (kind.endsWith(".paused")) return "pause";
  if (kind.endsWith(".resumed") || kind.endsWith(".recovered")) return "refresh";
  if (kind.endsWith(".created") || kind.endsWith(".queued") || kind.endsWith(".started")) return "play";
  return "info";
}
