// The Agents screens' state: the gallery, the agent that is open and the tab it is on.
import {
  type AgentCard,
  type AgentDetail,
  type Template,
  agentAddTemplate,
  agentGet,
  agentImport,
  agentTemplates,
  agentsList,
  errorText,
  teamAddTemplate,
  teamTemplates,
  teamsList,
  type TeamTemplate,
  type TeamView,
} from "../api.ts";
import { toast } from "./toast.svelte.ts";
import { t as tr } from "../i18n/index.ts";

export const AGENT_TABS = ["chat", "definition", "skills", "versions", "evals", "memory"] as const;
export type AgentTab = (typeof AGENT_TABS)[number];

class AgentsStore {
  cards = $state<AgentCard[]>([]);
  templates = $state<Template[]>([]);
  teams = $state<TeamView[]>([]);
  teamTemplates = $state<TeamTemplate[]>([]);
  /** Which gallery is showing, and the team that is open (null for the gallery). */
  section = $state<"agents" | "teams">("agents");
  teamSlug = $state<string | null>(null);
  /** The team the open chat talks to: its lead is the agent that is open. */
  chatTeam = $state<string | null>(null);
  loaded = $state(false);
  error = $state<string | null>(null);
  /** The agent that is open, or null for the gallery. */
  slug = $state<string | null>(null);
  tab = $state<AgentTab>("chat");
  detail = $state<AgentDetail | null>(null);
  /** The version being looked at (the newest unless a person chose an older one on the Versions tab). */
  viewing = $state<number | null>(null);
  /** A chat to open straight away (the conversation of a run). */
  startConversation = $state<string | null>(null);

  async refresh(): Promise<void> {
    try {
      [this.cards, this.templates, this.teams, this.teamTemplates] = await Promise.all([
        agentsList(false),
        agentTemplates(),
        teamsList(),
        teamTemplates(),
      ]);
      this.error = null;
    } catch (e) {
      this.error = errorText(e);
    } finally {
      this.loaded = true;
    }
  }

  /** Open an agent, or the gallery when `slug` is null. */
  async open(slug: string | null, tab: AgentTab = "chat", conversation: string | null = null, team: string | null = null): Promise<void> {
    this.section = "agents";
    this.teamSlug = null;
    this.chatTeam = team;
    this.slug = slug;
    this.tab = tab;
    this.startConversation = conversation;
    this.viewing = null;
    this.detail = null;
    if (slug) await this.reload();
    else await this.refresh();
  }

  /** Read the open agent again (after a save, a rollback or an import). */
  async reload(): Promise<void> {
    if (!this.slug) return;
    try {
      this.detail = await agentGet(this.slug);
      this.error = null;
    } catch (e) {
      this.error = errorText(e);
    }
    await this.refresh();
  }

  async addTemplate(id: string): Promise<string | null> {
    try {
      const saved = await agentAddTemplate(id);
      toast.ok(saved.created ? tr("stores.agents.added", { slug: saved.slug }) : tr("stores.agents.isAlreadyHere", { slug: saved.slug }));
      await this.refresh();
      return saved.slug;
    } catch (e) {
      toast.err(errorText(e));
      return null;
    }
  }

  async importFile(file: string, rename?: string): Promise<string | null> {
    try {
      const saved = await agentImport(file, rename?.trim() || undefined);
      toast.ok(saved.created ? tr("stores.agents.imported", { slug: saved.slug }) : saved.changed ? tr("stores.agents.isNowAtV", { slug: saved.slug, version: saved.version }) : tr("stores.agents.isUnchanged", { slug: saved.slug }));
      await this.refresh();
      return saved.slug;
    } catch (e) {
      toast.err(errorText(e));
      return null;
    }
  }

  /** Open the Teams gallery, or one team. */
  async openTeams(team: string | null = null): Promise<void> {
    this.section = "teams";
    this.slug = null;
    this.detail = null;
    this.teamSlug = team;
    await this.refresh();
  }

  async addTeamTemplate(id: string): Promise<string | null> {
    try {
      const slug = await teamAddTemplate(id);
      toast.ok(tr("stores.agents.addedTheTeam", { slug }));
      await this.refresh();
      return slug;
    } catch (e) {
      toast.err(errorText(e));
      return null;
    }
  }

  team(slug: string): TeamView | undefined {
    return this.teams.find((t) => t.team.slug === slug);
  }

  card(slug: string): AgentCard | undefined {
    return this.cards.find((c) => c.slug === slug);
  }
}

export const agents = new AgentsStore();
