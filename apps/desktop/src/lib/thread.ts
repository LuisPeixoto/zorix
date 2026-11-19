// Turns the runs of a conversation into what the chat draws: the person's message, the agent's tool calls with their
// approvals in the flow, and the answer (or what went wrong). Pure, so it is easy to test.
import type { Approval, ThreadRun, ToolCall } from "./api.ts";

export type ThreadItem =
  | { kind: "user"; id: string; runId: string; text: string; at: number }
  | { kind: "tool"; id: string; runId: string; call: ToolCall; approval: Approval | null }
  | { kind: "answer"; id: string; runId: string; text: string; at: number; version: number | null; streaming: boolean }
  | { kind: "problem"; id: string; runId: string; tone: "err" | "muted"; title: string; text: string }
  | { kind: "waiting"; id: string; runId: string; text: string };

/** The text an answer shows right now: what is stored, or what is still arriving for the run that is writing. */
export interface Live {
  runId: string;
  text: string;
}

const WORKING = new Set(["created", "queued", "running", "waiting_for_model", "waiting_for_tool"]);

export function buildThread(runs: ThreadRun[], live: Live | null = null): ThreadItem[] {
  const items: ThreadItem[] = [];
  for (const { execution: run, tool_calls, approvals } of runs) {
    items.push({ kind: "user", id: `${run.id}:user`, runId: run.id, text: run.request.prompt, at: run.created_at });
    for (const call of tool_calls) {
      const approval = approvals.find((a) => a.call_id === call.call_id) ?? null;
      items.push({ kind: "tool", id: `${run.id}:${call.call_id}`, runId: run.id, call, approval });
    }
    const version = run.agent?.version ?? null;
    if (run.state === "completed" && run.result) {
      items.push({ kind: "answer", id: `${run.id}:answer`, runId: run.id, text: run.result.text, at: run.finished_at ?? run.updated_at, version, streaming: false });
    } else if (run.state === "failed") {
      items.push({ kind: "problem", id: `${run.id}:problem`, runId: run.id, tone: "err", title: "This run failed", text: run.failure?.message ?? "It stopped without saying why." });
    } else if (run.state === "cancelled") {
      items.push({ kind: "problem", id: `${run.id}:problem`, runId: run.id, tone: "muted", title: "Stopped", text: run.cancel_reason ?? "This run was cancelled." });
    } else if (run.state === "waiting_for_approval") {
      items.push({ kind: "waiting", id: `${run.id}:waiting`, runId: run.id, text: "Waiting for you to decide on the step above." });
    } else if (run.state === "paused") {
      items.push({ kind: "waiting", id: `${run.id}:waiting`, runId: run.id, text: "Paused." });
    } else if (WORKING.has(run.state)) {
      const text = live && live.runId === run.id ? live.text : "";
      items.push({ kind: "answer", id: `${run.id}:answer`, runId: run.id, text, at: run.updated_at, version, streaming: true });
    }
  }
  return items;
}

/** The newest run of a thread that has not finished, if any: the chat follows it. */
export function openRun(runs: ThreadRun[]): ThreadRun | null {
  for (let i = runs.length - 1; i >= 0; i--) {
    const state = runs[i].execution.state;
    if (!["completed", "failed", "cancelled"].includes(state)) return runs[i];
  }
  return null;
}

/** Whether the person can send another message now (nothing is still working or waiting on them). */
export function canSend(runs: ThreadRun[]): boolean {
  return openRun(runs) === null;
}
