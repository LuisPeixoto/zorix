// What is waiting for a person, in one list: tool approvals anywhere in a tree of runs, and tools or sign-ins MCP servers want a person to review.
// Pure functions, tested in inbox.test.ts.
import type { Approval, Execution, NeedsYou, ServerRow, TaskNeed, TreeApproval, WfWait } from "./api.ts";
import { shorten } from "./format.ts";

export type InboxItem =
  | {
      kind: "approval";
      id: string;
      /** The run the person started: the answer is given there, even when a sub-run asked. */
      root: string;
      rootPrompt: string;
      /** The run that waits (the root, or a sub-run). */
      execution: string;
      agent: string | null;
      depth: number;
      tool: string;
      /** What the call is about, in a few words: a path, an address, a channel. */
      target: string;
      reason: string;
      at: number;
      approval: Approval;
    }
  | { kind: "server"; id: string; server: string; why: "tools" | "login"; tools: number; at: number }
  | {
      /** A workflow stopped for a person: an approval, a question, or an action that asks first. */
      kind: "workflow";
      id: string;
      run: string;
      workflow: string;
      step: string;
      title: string;
      message: string;
      what: "approval" | "question" | "action";
      at: number;
    }
  | {
      /** A task that needs you: it is in Review, or its run failed or waits for an approval. */
      kind: "task";
      id: string;
      key: string;
      title: string;
      project: string;
      why: NeedsYou;
      at: number;
    };

const TARGET_KEYS = ["target", "path", "url", "channel", "repo", "query", "sql", "to", "name", "subject", "title", "content", "text", "message"];

/** What a tool call is about, taken from its arguments: the first of a few well-known keys, else the arguments in brief. */
export function targetOf(args: unknown): string {
  if (args && typeof args === "object") {
    const record = args as Record<string, unknown>;
    for (const key of TARGET_KEYS) {
      const value = record[key];
      if (typeof value === "string" && value.trim()) return shorten(value, 70);
    }
    const text = JSON.stringify(record);
    return text === "{}" ? "" : shorten(text, 70);
  }
  return typeof args === "string" ? shorten(args, 70) : "";
}

/** One item per pending approval found under the runs that wait. */
export function approvalItems(runs: readonly Pick<Execution, "id" | "request">[], byRoot: Readonly<Record<string, TreeApproval[]>>): InboxItem[] {
  return runs.flatMap((run) =>
    (byRoot[run.id] ?? [])
      .filter((t) => t.approval.status === "pending")
      .map((t): InboxItem => ({
        kind: "approval",
        id: t.approval.id,
        root: run.id,
        rootPrompt: run.request.prompt,
        execution: t.execution,
        agent: t.agent,
        depth: t.depth,
        tool: t.approval.tool,
        target: targetOf(t.approval.arguments),
        reason: t.approval.reason,
        at: t.approval.requested_at,
        approval: t.approval,
      })),
  );
}

/** One item per enabled server that wants a person: tools to review, or a sign-in. */
export function serverItems(servers: readonly ServerRow[]): InboxItem[] {
  return servers
    .filter((s) => s.enabled && (s.tools_to_review > 0 || s.state === "needs_login"))
    .map((s): InboxItem => ({ kind: "server", id: `server:${s.name}`, server: s.name, why: s.state === "needs_login" ? "login" : "tools", tools: s.tools_to_review, at: 0 }));
}

/** One item per wait of a workflow that a person must answer (timers are not asked of anyone). */
export function workflowItems(waits: readonly WfWait[], names: Readonly<Record<string, string>> = {}): InboxItem[] {
  return waits
    .filter((w) => w.status === "pending" && w.kind !== "timer")
    .map((w): InboxItem => ({
      kind: "workflow",
      id: w.id,
      run: w.run,
      workflow: names[w.workflow] ?? "",
      step: w.step,
      title: w.title,
      message: w.message,
      what: w.kind as "approval" | "question" | "action",
      at: w.created_at,
    }));
}

/** One item per task that needs a person. */
export function taskItems(needs: readonly TaskNeed[]): InboxItem[] {
  return needs.map((n): InboxItem => ({ kind: "task", id: `task:${n.key}`, key: n.key, title: n.title, project: n.project_name, why: n.needs, at: n.at }));
}

/** Oldest ask first, as the design system says: what has a time (approvals, workflow waits) by time, then servers (which have none) by name. */
export function oldestFirst(items: readonly InboxItem[]): InboxItem[] {
  const timed = (i: InboxItem) => i.kind !== "server";
  return [...items].sort((a, b) => {
    if (timed(a) !== timed(b)) return timed(a) ? -1 : 1;
    if (timed(a)) return a.at - b.at || a.id.localeCompare(b.id);
    return a.id.localeCompare(b.id);
  });
}
