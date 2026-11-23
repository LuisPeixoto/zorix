// Which projects screen is open, and what the screens share: the projects, the open board, who the agents are (for names and
// faces), and the tasks that need a person (for the Inbox and the bell). The screens fetch the rest themselves.
import {
  type AgentCard,
  type Board,
  type ProjectCard,
  type TaskIndexRow,
  type TaskNeed,
  type TeamView,
  type Who,
  RUN_GOING,
  type TaskState,
  agentsList,
  errorText,
  projectBoard,
  projectsOverview,
  taskMove,
  tasksIndex,
  tasksNeedingYou,
  teamsList,
} from "../api.ts";
import { toast } from "./toast.svelte.ts";
import { t as tr } from "../i18n/index.ts";

export type ProjectSection = "list" | "project";
export const PROJECT_TABS = ["board", "list", "timeline", "deliverables", "history"] as const;
export type ProjectTab = (typeof PROJECT_TABS)[number];

class ProjectsStore {
  section = $state<ProjectSection>("list");
  /** The project that is open. */
  key = $state<string | null>(null);
  tab = $state<ProjectTab>("board");
  /** The task open in the drawer. */
  taskKey = $state<string | null>(null);
  showArchived = $state(false);
  cards = $state<ProjectCard[]>([]);
  board = $state<Board | null>(null);
  agents = $state<AgentCard[]>([]);
  teams = $state<TeamView[]>([]);
  needs = $state<TaskNeed[]>([]);
  index = $state<TaskIndexRow[]>([]);
  error = $state("");
  /** A move that would stop a run that is going, waiting for a yes. */
  pendingStop = $state<{ key: string; to: TaskState; index: number | null; note: string | null } | null>(null);

  /**
   * Move a task. A move that would stop a run asks first (`pendingStop`); anything else that goes wrong is said, and the board is
   * read again so it shows what is true.
   */
  async move(key: string, to: TaskState, index: number | null = null, note: string | null = null, cancel = false): Promise<boolean> {
    try {
      await taskMove(key, { to, index, note, cancel_run: cancel });
      this.pendingStop = null;
      await Promise.all([this.refreshBoard(), this.refreshNeeds()]);
      return true;
    } catch (e) {
      const message = errorText(e);
      if (message.includes(RUN_GOING) && !cancel) {
        this.pendingStop = { key, to, index, note };
      } else {
        toast.err(message.replace(RUN_GOING, "").trim());
      }
      await this.refreshBoard();
      return false;
    }
  }

  async refreshList(): Promise<void> {
    try {
      this.cards = await projectsOverview(this.showArchived);
      this.error = "";
    } catch (e) {
      this.error = errorText(e);
    }
  }

  async refreshBoard(): Promise<void> {
    if (!this.key) return;
    try {
      this.board = await projectBoard(this.key);
      this.error = "";
    } catch (e) {
      this.error = errorText(e);
    }
  }

  async refreshNeeds(): Promise<void> {
    this.needs = await tasksNeedingYou().catch(() => []);
  }

  async refreshIndex(): Promise<void> {
    this.index = await tasksIndex().catch(() => []);
  }

  async loadPeople(): Promise<void> {
    const [agents, teams] = await Promise.all([agentsList().catch(() => []), teamsList().catch(() => [])]);
    this.agents = agents;
    this.teams = teams;
  }

  agentName = (slug: string): string => this.agents.find((a) => a.slug === slug)?.name ?? this.teams.find((t) => t.team.slug === slug)?.team.name ?? slug;

  /** What a person sees of who has a task: an agent's face, a team's, or a round avatar for you. */
  face(who: Who | null): { human: boolean; emoji: string; color: string; label: string } {
    if (!who) return { human: false, emoji: "·", color: "muted", label: tr("common.nobody") };
    if (who.type === "you") return { human: true, emoji: "", color: "muted", label: tr("common.you") };
    if (who.type === "agent") {
      const agent = this.agents.find((a) => a.slug === who.slug);
      return { human: false, emoji: agent?.emoji ?? "🤖", color: agent?.color ?? "accent", label: agent?.name ?? who.slug };
    }
    const team = this.teams.find((t) => t.team.slug === who.slug);
    return { human: false, emoji: team?.team.avatar.emoji ?? "👥", color: team?.team.avatar.color ?? "accent", label: team?.team.name ?? who.slug };
  }

  openList(): void {
    this.section = "list";
    this.key = null;
    this.board = null;
    this.taskKey = null;
    void this.refreshList();
  }

  openProject(key: string, tab: ProjectTab = "board", task: string | null = null): void {
    if (this.key !== key) this.board = null;
    this.section = "project";
    this.key = key;
    this.tab = tab;
    this.taskKey = task;
  }

  /** A task is open in its project, wherever it was asked for from. */
  openTask(taskKey: string): void {
    const project = taskKey.slice(0, taskKey.lastIndexOf("-"));
    this.openProject(project, this.key === project ? this.tab : "board", taskKey);
  }
}

export const projects = new ProjectsStore();
