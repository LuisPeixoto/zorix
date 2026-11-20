// What a call to `agents.delegate` shows in the chat: which sub-runs it started and how each one stands. Pure, so it is tested.
import type { Execution, ToolCall } from "./api.ts";
import type { ChildRow } from "./ui/DelegationCard.svelte";

export const DELEGATE = "agents.delegate";

/** The ids of the sub-runs a delegate call started: from the answer it stored (started, or settled with results). */
export function childIds(call: Pick<ToolCall, "result">): string[] {
  const r = call.result as { delegated?: { child?: string }[]; results?: { child_id?: string; child?: string }[] } | null | undefined;
  const ids = [...(r?.delegated ?? []).map((d) => d.child), ...(r?.results ?? []).map((d) => d.child_id ?? d.child)];
  return [...new Set(ids.filter((id): id is string => typeof id === "string"))];
}

/** How a sub-run stands, in the words of the card. */
export function rowState(run: Execution, waitingForYou: boolean): ChildRow["state"] {
  if (run.state === "completed") return "completed";
  if (run.state === "failed") return "failed";
  if (run.state === "cancelled") return "cancelled";
  if (waitingForYou || run.state === "waiting_for_approval") return "waiting";
  if (run.state === "created" || run.state === "queued") return "starting";
  return "working";
}

export interface Look {
  name: string;
  emoji: string;
  color: string;
}

/** The rows of the card for one delegate call. `runs` are the executions of the tree; `needsYou` the sub-runs with an approval open. */
export function childRows(
  call: Pick<ToolCall, "arguments" | "result">,
  runs: Execution[],
  needsYou: ReadonlySet<string>,
  look: (run: Execution) => Look,
  live: Readonly<Record<string, string>> = {},
): ChildRow[] {
  const byId = new Map(runs.map((r) => [r.id, r]));
  const tasks = ((call.arguments as { tasks?: { task?: string }[] } | null)?.tasks ?? []).map((t) => t.task ?? "");
  const started = ((call.result as { delegated?: { child?: string; task?: string }[] } | null)?.delegated ?? []);
  return childIds(call).flatMap((id, i) => {
    const run = byId.get(id);
    if (!run) return [];
    const face = look(run);
    const task = started.find((d) => d.child === id)?.task ?? tasks[i] ?? run.request.prompt;
    const state = rowState(run, needsYou.has(id));
    const failed = state === "failed" || state === "cancelled";
    return [
      {
        id,
        agent: run.agent?.slug ?? "agent",
        name: face.name,
        emoji: face.emoji,
        color: face.color,
        task,
        state,
        steps: run.iteration,
        cost_micros: run.usage.cost_micros,
        answer: state === "completed" ? (run.result?.text ?? null) : null,
        error: failed ? (run.failure?.message ?? run.cancel_reason ?? (state === "cancelled" ? "Stopped before it finished." : "It did not finish.")) : null,
        live: state === "working" ? (live[id] ?? null) : null,
      },
    ];
  });
}
