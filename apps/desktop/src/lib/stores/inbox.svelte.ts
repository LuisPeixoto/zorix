// What waits for a person. Approvals are fed by the runs store each time it refreshes; servers that want a review come from the Integrations store.
import type { Execution, TaskNeed, TreeApproval, WfWait } from "../api.ts";
import { type InboxItem, approvalItems, oldestFirst, serverItems, taskItems, workflowItems } from "../inbox.ts";
import { integrations } from "./integrations.svelte.ts";
import { workflows } from "./workflows.svelte.ts";

class InboxStore {
  approvals = $state<InboxItem[]>([]);
  /** Workflows that stopped for a person. */
  waits = $state<InboxItem[]>([]);
  /** Tasks that need you. */
  tasks = $state<InboxItem[]>([]);

  /** Oldest ask first. */
  get all(): InboxItem[] {
    return oldestFirst([...this.approvals, ...this.waits, ...this.tasks, ...serverItems(integrations.servers)]);
  }

  get count(): number {
    return this.approvals.length + this.waits.length + this.tasks.length + serverItems(integrations.servers).length;
  }

  setWaits(waits: readonly WfWait[]): void {
    const names = Object.fromEntries((workflows.overview?.cards ?? []).map((c) => [c.workflow.id, c.workflow.slug]));
    this.waits = workflowItems(waits, names);
  }

  setTasks(needs: readonly TaskNeed[]): void {
    this.tasks = taskItems(needs);
  }

  setApprovals(runs: readonly Execution[], byRoot: Record<string, TreeApproval[]>): void {
    this.approvals = approvalItems(runs, byRoot);
  }
}

export const inbox = new InboxStore();
