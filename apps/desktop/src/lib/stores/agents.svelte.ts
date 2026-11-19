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
} from "../api.ts";
import { toast } from "./toast.svelte.ts";

export const AGENT_TABS = ["chat", "definition", "skills", "versions", "evals", "memory"] as const;
export type AgentTab = (typeof AGENT_TABS)[number];

class AgentsStore {
  cards = $state<AgentCard[]>([]);
  templates = $state<Template[]>([]);
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
      [this.cards, this.templates] = await Promise.all([agentsList(false), agentTemplates()]);
      this.error = null;
    } catch (e) {
      this.error = errorText(e);
    } finally {
      this.loaded = true;
    }
  }

  /** Open an agent, or the gallery when `slug` is null. */
  async open(slug: string | null, tab: AgentTab = "chat", conversation: string | null = null): Promise<void> {
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
      toast.ok(saved.created ? `Added ${saved.slug}.` : `${saved.slug} is already here.`);
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
      toast.ok(saved.created ? `Imported ${saved.slug}.` : saved.changed ? `${saved.slug} is now at v${saved.version}.` : `${saved.slug} is unchanged.`);
      await this.refresh();
      return saved.slug;
    } catch (e) {
      toast.err(errorText(e));
      return null;
    }
  }

  card(slug: string): AgentCard | undefined {
    return this.cards.find((c) => c.slug === slug);
  }
}

export const agents = new AgentsStore();
