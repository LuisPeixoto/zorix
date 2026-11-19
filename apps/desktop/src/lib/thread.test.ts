import assert from "node:assert/strict";
import { test } from "node:test";

import type { Approval, ThreadRun, ToolCall } from "./api.ts";
import { buildThread, canSend, openRun } from "./thread.ts";

function run(id: string, state: string, extra: Record<string, unknown> = {}, calls: ToolCall[] = [], approvals: Approval[] = []): ThreadRun {
  return {
    execution: {
      id,
      request: { prompt: `ask ${id}`, instructions: null, model: "demo" },
      state,
      created_at: 1,
      updated_at: 2,
      finished_at: 3,
      result: null,
      failure: null,
      cancel_reason: null,
      agent: { agent: "a", slug: "calc", version: 2, hash: "h" },
      ...extra,
    },
    tool_calls: calls,
    approvals,
  } as unknown as ThreadRun;
}

const call = (id: string, tool = "math.evaluate"): ToolCall => ({ call_id: id, tool, arguments: {}, status: "succeeded", attempts: 1, error: null });
const approval = (callId: string): Approval => ({ id: `ap-${callId}`, call_id: callId, status: "pending" }) as unknown as Approval;

test("a finished run is the person's message, its tool calls and the answer, in that order", () => {
  const items = buildThread([run("r1", "completed", { result: { text: "391", structured: null } }, [call("c1"), call("c2", "clock.now")])]);
  assert.deepEqual(items.map((i) => i.kind), ["user", "tool", "tool", "answer"]);
  const answer = items[3];
  assert.equal(answer.kind === "answer" && answer.text, "391");
  assert.equal(answer.kind === "answer" && answer.version, 2);
  assert.equal(answer.kind === "answer" && answer.streaming, false);
});

test("an approval sits on the tool call it is about", () => {
  const items = buildThread([run("r1", "waiting_for_approval", {}, [call("c1"), call("c2")], [approval("c2")])]);
  const tools = items.filter((i) => i.kind === "tool");
  assert.equal(tools[0].kind === "tool" && tools[0].approval, null);
  assert.equal(tools[1].kind === "tool" && tools[1].approval?.id, "ap-c2");
  assert.equal(items[items.length - 1].kind, "waiting");
});

test("a run that is still working shows an answer that is being written, with the live text only for its own run", () => {
  const runs = [run("r1", "completed", { result: { text: "done", structured: null } }), run("r2", "running")];
  const items = buildThread(runs, { runId: "r2", text: "Thinking about" });
  const last = items[items.length - 1];
  assert.equal(last.kind === "answer" && last.streaming, true);
  assert.equal(last.kind === "answer" && last.text, "Thinking about");
  const other = buildThread(runs, { runId: "someone-else", text: "x" });
  assert.equal(other[other.length - 1].kind === "answer" && (other[other.length - 1] as { text: string }).text, "");
});

test("failures and cancellations say what happened", () => {
  const failed = buildThread([run("r1", "failed", { failure: { kind: "model", message: "The model refused.", limit: null } })]);
  assert.deepEqual([failed[1].kind, failed[1].kind === "problem" && failed[1].text], ["problem", "The model refused."]);
  const cancelled = buildThread([run("r1", "cancelled", { cancel_reason: "Cancelled from the desktop app" })]);
  assert.equal(cancelled[1].kind === "problem" && cancelled[1].tone, "muted");
  const silent = buildThread([run("r1", "failed")]);
  assert.match(silent[1].kind === "problem" ? silent[1].text : "", /without saying why/);
});

test("the person can write again only when nothing is open", () => {
  const done = run("r1", "completed", { result: { text: "x", structured: null } });
  assert.equal(canSend([done]), true);
  assert.equal(canSend([done, run("r2", "running")]), false);
  assert.equal(canSend([run("r3", "waiting_for_approval")]), false);
  assert.equal(canSend([run("r4", "failed"), run("r5", "cancelled")]), true);
  assert.equal(openRun([done, run("r2", "paused")])?.execution.id, "r2");
  assert.equal(canSend([]), true);
});
