// Turning engine data into words and short strings. Pure functions, tested in format.test.ts.
import type { ExecEvent, ExecutionState } from "./api.ts";

export type Tone = "ok" | "warn" | "err" | "info" | "muted";

const LABELS: Record<ExecutionState, string> = {
  created: "Created",
  queued: "Queued",
  running: "Running",
  waiting_for_model: "Thinking",
  waiting_for_tool: "Using a tool",
  waiting_for_approval: "Needs your approval",
  paused: "Paused",
  completed: "Completed",
  failed: "Failed",
  cancelled: "Cancelled",
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
  if (ms < 1000) return `${Math.round(ms)} ms`;
  const seconds = ms / 1000;
  if (seconds < 60) return `${seconds < 10 ? seconds.toFixed(1) : Math.round(seconds)} s`;
  const minutes = Math.floor(seconds / 60);
  return `${minutes} min ${Math.round(seconds % 60)} s`;
}

export function ago(at: number, now: number = Date.now()): string {
  const seconds = Math.max(0, Math.round((now - at) / 1000));
  if (seconds < 5) return "just now";
  if (seconds < 60) return `${seconds} s ago`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  return `${Math.floor(hours / 24)} d ago`;
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

/** A person-friendly line for each kind of event. */
export function describeEvent(event: ExecEvent): EventView {
  const p = event.payload;
  const tool = String(p.tool ?? "");
  switch (event.kind) {
    case "execution.created":
      return { title: "Created", detail: `model ${p.model}`, tone: "muted" };
    case "execution.queued":
      return { title: "Queued", detail: "", tone: "muted" };
    case "execution.started":
      return { title: "Started", detail: "", tone: "info" };
    case "execution.resumed":
      return { title: "Resumed", detail: "", tone: "info" };
    case "execution.paused":
      return { title: "Paused", detail: "", tone: "muted" };
    case "execution.recovered":
      return { title: "Recovered after a restart", detail: "", tone: "info" };
    case "execution.model.requested":
      return { title: "Asked the model", detail: `step ${p.iteration}`, tone: "info" };
    case "execution.model.responded": {
      const calls = Number(p.tool_calls ?? 0);
      return { title: "Model answered", detail: calls ? `wants ${calls} tool call${calls > 1 ? "s" : ""}` : "final answer", tone: "info" };
    }
    case "execution.model.retrying":
      return { title: "Model failed, trying again", detail: `${p.error} (in ${p.delay_ms} ms)`, tone: "warn" };
    case "execution.tool.requested":
      return { title: `Tool requested: ${tool}`, detail: shorten(JSON.stringify(p.arguments ?? {}), 140), tone: "info" };
    case "execution.tool.started":
      return { title: `Running ${tool}`, detail: Number(p.attempt) > 1 ? `attempt ${p.attempt}` : "", tone: "info" };
    case "execution.tool.completed":
      return { title: `Finished ${tool}`, detail: duration(Number(p.duration_ms ?? 0)), tone: "ok" };
    case "execution.tool.failed":
      return { title: `${tool} failed`, detail: shorten(String(p.error ?? p.reason ?? ""), 160), tone: "err" };
    case "execution.tool.denied":
      return { title: `${tool} was refused`, detail: shorten(`${p.reason ?? ""} (by ${p.by})`, 160), tone: "warn" };
    case "execution.waiting_approval":
      return { title: `Waiting for approval: ${tool}`, detail: String(p.reason ?? ""), tone: "warn" };
    case "execution.approval.granted":
      return { title: "Approved", detail: `by ${p.decided_by}${p.note ? ` – ${p.note}` : ""}`, tone: "ok" };
    case "execution.approval.denied":
      return { title: "Refused", detail: `by ${p.decided_by}${p.note ? ` – ${p.note}` : ""}`, tone: "warn" };
    case "execution.result.rejected":
      return { title: "Answer rejected", detail: String(p.reason ?? ""), tone: "warn" };
    case "execution.completed":
      return { title: "Completed", detail: `${p.iterations} step${Number(p.iterations) === 1 ? "" : "s"}`, tone: "ok" };
    case "execution.failed":
      return { title: "Failed", detail: String(p.message ?? ""), tone: "err" };
    case "execution.cancelled":
      return { title: "Cancelled", detail: String(p.reason ?? ""), tone: "muted" };
    default:
      return { title: event.kind.replace("execution.", ""), detail: "", tone: "muted" };
  }
}

/** Events worth a row in the timeline. Bookkeeping is hidden to keep the story readable. */
export const isNoise = (event: ExecEvent): boolean => ["execution.progress", "execution.checkpoint"].includes(event.kind);
