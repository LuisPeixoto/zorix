// Which workflows screen is open, and the list with its figures. The screens fetch what they show; this keeps what is shared:
// where the person is, and the overview that the sidebar and the Inbox also need.
import { type WfOverview, errorText, workflowsOverview } from "../api.ts";

export type WorkflowSection = "list" | "editor" | "run";

class WorkflowsStore {
  section = $state<WorkflowSection>("list");
  /** The workflow open in the editor (or the one a run belongs to). */
  id = $state<string | null>(null);
  /** The run open in the run view. */
  runId = $state<string | null>(null);
  overview = $state<WfOverview | null>(null);
  error = $state("");
  /** The editor opens on this panel first (for screenshots): `versions`. */
  panel = $state<string | null>(null);

  async refresh(): Promise<void> {
    try {
      this.overview = await workflowsOverview();
      this.error = "";
    } catch (e) {
      this.error = errorText(e);
    }
  }

  openList(): void {
    this.section = "list";
    this.id = null;
    this.runId = null;
    this.panel = null;
    void this.refresh();
  }

  openEditor(id: string, panel: string | null = null): void {
    this.section = "editor";
    this.id = id;
    this.runId = null;
    this.panel = panel;
  }

  openRun(run: string, workflow: string | null = null): void {
    this.section = "run";
    this.runId = run;
    if (workflow) this.id = workflow;
  }
}

export const workflows = new WorkflowsStore();
