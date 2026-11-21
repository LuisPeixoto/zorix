// What waits for a person. Approvals are fed by the runs store each time it refreshes; servers that want a review come from the Integrations store.
import type { Execution, TreeApproval } from "../api.ts";
import { type InboxItem, approvalItems, oldestFirst, serverItems } from "../inbox.ts";
import { integrations } from "./integrations.svelte.ts";

class InboxStore {
  approvals = $state<InboxItem[]>([]);

  /** Oldest ask first. */
  get all(): InboxItem[] {
    return oldestFirst([...this.approvals, ...serverItems(integrations.servers)]);
  }

  get count(): number {
    return this.approvals.length + serverItems(integrations.servers).length;
  }

  setApprovals(runs: readonly Execution[], byRoot: Record<string, TreeApproval[]>): void {
    this.approvals = approvalItems(runs, byRoot);
  }
}

export const inbox = new InboxStore();
