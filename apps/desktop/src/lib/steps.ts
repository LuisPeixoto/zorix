// The Steps tab of a run: the model and tool steps in the order they happened, each tool step joined to its call. Pure, tested in steps.test.ts.
import type { Summary, ToolCall } from "./api.ts";

export type StepItem =
  | { kind: "model"; title: string; ms: number }
  | { kind: "tool"; title: string; ms: number; call: ToolCall | null };

/** Steps in order. The nth tool step belongs to the nth tool call; a step with no call (not recorded yet) has `call: null`. */
export function runSteps(steps: Summary["steps"], calls: readonly ToolCall[]): StepItem[] {
  let nextCall = 0;
  return steps.map((step): StepItem => {
    if (step.kind === "model") return { kind: "model", title: step.provider ? `${step.provider}/${step.name}` : step.name, ms: step.duration_ms };
    const call = calls[nextCall] ?? null;
    nextCall += 1;
    return { kind: "tool", title: call?.tool ?? step.name, ms: step.duration_ms, call };
  });
}

/** How many different providers served the model steps. */
export function providerCount(steps: Summary["steps"]): number {
  return new Set(steps.filter((s) => s.kind === "model").map((s) => s.provider ?? s.name)).size;
}
