// Which workforce screen is open, and what the screens share: the organisation, the open worker, the activity and the notices. The
// screens fetch the rest themselves.
import {
  type ActivityLine,
  type AgentCard,
  type Notice,
  type Overview,
  type WorkerDetail,
  type WorkerFields,
  agentsList,
  errorText,
  workerDetail,
  workerEdit,
  workforceActivity,
  workforceMarkSeen,
  workforceNotices,
  workforceOverview,
} from "../api.ts";
import { toast } from "./toast.svelte.ts";

export type WorkforceSection = "overview" | "worker";
export const WORKFORCE_TABS = ["chart", "list", "activity"] as const;
export type WorkforceTab = (typeof WORKFORCE_TABS)[number];

class WorkforceStore {
  section = $state<WorkforceSection>("overview");
  tab = $state<WorkforceTab>("chart");
  /** The worker selected in the panel, or open on its own page. */
  selected = $state<string | null>(null);
  overview = $state<Overview | null>(null);
  detail = $state<WorkerDetail | null>(null);
  activity = $state<ActivityLine[]>([]);
  notices = $state<Notice[]>([]);
  /** Every agent, for the names and faces of the hire form. */
  agents = $state<AgentCard[]>([]);
  error = $state("");

  async refresh(): Promise<void> {
    try {
      this.overview = await workforceOverview();
      this.error = "";
      if (this.selected && !this.overview.workers.some((w) => w.worker.agent === this.selected) && this.selected !== "you") {
        this.selected = null;
      }
    } catch (e) {
      this.error = errorText(e);
    }
  }

  async refreshActivity(): Promise<void> {
    this.activity = await workforceActivity(150).catch(() => this.activity);
  }

  async refreshDetail(): Promise<void> {
    if (this.section !== "worker" || !this.selected) return;
    try {
      this.detail = await workerDetail(this.selected);
      this.error = "";
    } catch (e) {
      this.error = errorText(e);
    }
  }

  async refreshNotices(): Promise<void> {
    this.notices = await workforceNotices().catch(() => this.notices);
  }

  async markSeen(): Promise<void> {
    try {
      await workforceMarkSeen();
      this.notices = [];
    } catch (e) {
      toast.err(errorText(e));
    }
  }

  async loadAgents(): Promise<void> {
    this.agents = await agentsList().catch(() => this.agents);
  }

  /** Change one worker. What is wrong (a worker freer than its manager, say) is said in words and the screen stays as it was. */
  async edit(agent: string, change: (fields: WorkerFields) => WorkerFields): Promise<boolean> {
    const view = this.overview?.workers.find((w) => w.worker.agent === agent);
    if (!view) return false;
    const w = view.worker;
    const current: WorkerFields = {
      role: w.role,
      department: w.department,
      reports_to: w.reports_to,
      manager: w.manager,
      autonomy: w.autonomy,
      weekly_budget_micros: w.weekly_budget_micros,
      task_time_ms: w.task_time_ms,
      still_ask: w.still_ask,
      schedule_enabled: w.schedule.enabled,
    };
    try {
      await workerEdit(agent, change(current));
      await Promise.all([this.refresh(), this.refreshDetail()]);
      return true;
    } catch (e) {
      toast.err(errorText(e));
      return false;
    }
  }

  agentOf = (slug: string): AgentCard | undefined => this.agents.find((a) => a.slug === slug);

  openOverview(tab: WorkforceTab = this.tab, worker: string | null = this.selected): void {
    this.section = "overview";
    this.tab = tab;
    this.selected = worker;
    this.detail = null;
    void this.refresh();
  }

  openWorker(slug: string): void {
    if (this.selected !== slug) this.detail = null;
    this.section = "worker";
    this.selected = slug;
    void this.refreshDetail();
  }
}

export const workforce = new WorkforceStore();
