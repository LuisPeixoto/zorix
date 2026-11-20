<script lang="ts">
  import { onMount } from "svelte";

  import { store } from "./lib/stores/runs.svelte.ts";
  import { theme } from "./lib/stores/theme.svelte.ts";
  import NewRun from "./lib/components/NewRun.svelte";
  import Memory from "./lib/components/Memory.svelte";
  import Agents from "./lib/components/agents/Agents.svelte";
  import { AGENT_TABS, type AgentTab, agents } from "./lib/stores/agents.svelte.ts";
  import Integrations from "./lib/components/integrations/Integrations.svelte";
  import { integrations } from "./lib/stores/integrations.svelte.ts";
  import Providers from "./lib/components/Providers.svelte";
  import DesignSystem from "./lib/views/DesignSystem.svelte";
  import RunView from "./lib/components/RunView.svelte";
  import Sidebar from "./lib/components/shell/Sidebar.svelte";
  import Toolbar from "./lib/components/shell/Toolbar.svelte";
  import { type Command, CommandPalette, Toasts } from "./lib/ui/index.ts";
  import { shorten } from "./lib/format.ts";

  onMount(() => {
    theme.init();
    void store.init().then(() => {
      void integrations.init();
      void agents.refresh();
      // For screenshots and reviews: open straight to a screen.
      const [where, forced] = (store.info?.start_view || import.meta.env.VITE_START_VIEW || "").split("@");
      if (forced === "light" || forced === "dark") theme.preview(forced);
      const [start, tab] = where.split(":");
      if (start === "design") store.showDesign();
      if (start === "providers") {
        store.startTab = tab || "facts";
        store.showProviders();
      }
      if (start === "integrations") store.showIntegrations(tab || "connected");
      if (start === "server") {
        store.showIntegrations("connected");
        void integrations.refresh().then(() => integrations.open(tab || integrations.servers[0]?.name));
      }
      if (start === "agents") {
        const [slug, wanted, conversation] = tab ? tab.split("/") : [];
        store.showAgents(slug || null, AGENT_TABS.includes(wanted as AgentTab) ? (wanted as AgentTab) : "chat", conversation || null);
      }
      if (start === "teams") {
        // `teams` shows the gallery; `teams:SLUG` opens a team; `teams:SLUG/chat[/CONVERSATION]` talks to it through its lead.
        const [slug, how, conversation] = tab ? tab.split("/") : [];
        if (how === "chat") {
          void agents.refresh().then(() => {
            const lead = agents.team(slug)?.team.lead;
            if (lead) store.showAgents(lead, "chat", conversation || null, slug);
          });
        } else store.showTeams(slug || null);
      }
      if (start === "memory") {
        store.startTab = tab || "facts";
        store.showMemory();
      }
      const shown = store.runs[Number(tab || 0)];
      if (start === "run" && shown) void store.select(shown.id);
    });
  });

  // The design system opens with Ctrl/⌘ + Shift + D in any build, so it is easy to show.
  const commands = $derived<Command[]>([
    { id: "new", label: "New run", group: "Actions", icon: "plus", keywords: "start create", run: () => { store.continuing = null; store.showRuns(); store.showForm(); } },
    { id: "agents", label: "Agents", group: "Go to", icon: "bot", keywords: "assistant worker skills evals versions chat templates", run: () => store.showAgents() },
    { id: "teams", label: "Teams", group: "Go to", icon: "users", keywords: "lead members delegate orchestrate group crew", run: () => store.showTeams() },
    ...agents.teams.map((t) => ({
      id: `team-${t.team.slug}`,
      label: `Team: ${t.team.name}`,
      group: "Teams",
      icon: "users",
      hint: `${t.team.members.length + 1} agents`,
      keywords: `${t.team.slug} ${t.team.lead}`,
      run: () => store.showTeams(t.team.slug),
    })),
    ...agents.cards.map((card) => ({
      id: `agent-${card.slug}`,
      label: `Agent: ${card.name}`,
      group: "Agents",
      icon: "bot",
      hint: `v${card.version}`,
      keywords: card.slug,
      run: () => store.showAgents(card.slug),
    })),
    { id: "memory", label: "Memory", group: "Go to", icon: "book", keywords: "facts remember knowledge", run: () => store.showMemory() },
    { id: "integrations", label: "Integrations", group: "Go to", icon: "plug", keywords: "mcp servers connections catalogue github slack database", run: () => store.showIntegrations() },
    { id: "catalogue", label: "Browse the server catalogue", group: "Integrations", icon: "grid", keywords: "mcp add", run: () => store.showIntegrations("catalogue") },
    { id: "connections", label: "Connections and sandbox", group: "Integrations", icon: "link", keywords: "github gitlab jira slack discord postgres sqlite webhook python", run: () => store.showIntegrations("connections") },
    ...integrations.servers.map((server) => ({
      id: `server-${server.name}`,
      label: `Server: ${server.name}`,
      group: "Integrations",
      icon: "plug",
      hint: server.state.replaceAll("_", " "),
      run: () => { store.showIntegrations("connected"); void integrations.open(server.name); },
    })),
    { id: "providers", label: "Providers & keys", group: "Go to", icon: "key", keywords: "models api settings", run: () => store.showProviders() },
    { id: "theme", label: "Change theme", group: "Actions", icon: "sun", hint: "light, dark, system", keywords: "dark light appearance", run: () => theme.cycle() },
    ...(import.meta.env.DEV ? [{ id: "design", label: "Design system", group: "Go to", icon: "layers", run: () => store.showDesign() }] : []),
    ...store.runs.slice(0, 12).map((run) => ({
      id: `run-${run.id}`,
      label: shorten(run.request.prompt, 70),
      group: "Runs",
      icon: run.state === "waiting_for_approval" ? "shield" : "history",
      hint: run.state.replaceAll("_", " "),
      run: () => { store.showRuns(); void store.select(run.id); },
    })),
  ]);

  function onkey(event: KeyboardEvent) {
    if ((event.metaKey || event.ctrlKey) && !event.shiftKey && event.key.toLowerCase() === "k") {
      event.preventDefault();
      store.palette = !store.palette;
      return;
    }
    if ((event.metaKey || event.ctrlKey) && event.shiftKey && event.key.toLowerCase() === "d") {
      event.preventDefault();
      store.showDesign();
    }
  }
</script>

<svelte:window onkeydown={onkey} />

<div class="flex h-full flex-col">
  <Toolbar />
  <div class="flex min-h-0 flex-1">
    <Sidebar />
    <main class="page-wash min-w-0 flex-1 overflow-hidden bg-bg">
      {#if store.info?.error}
        <div class="mx-auto max-w-xl p-10" role="alert">
          <h2 class="text-2xl font-semibold text-err">ZORIX could not start</h2>
          <p class="selectable mt-2">{store.info.error}</p>
        </div>
      {:else if store.view === "design"}
        <DesignSystem />
      {:else if store.view === "agents"}
        <Agents />
      {:else if store.view === "memory"}
        <Memory />
      {:else if store.view === "integrations"}
        <Integrations />
      {:else if store.view === "providers"}
        <Providers />
      {:else if store.selectedId}
        {#if store.error}<p class="border-b border-line bg-err/10 px-6 py-2 text-err" role="alert">{store.error}</p>{/if}
        <RunView />
      {:else}
        <NewRun />
      {/if}
    </main>
  </div>
</div>

<CommandPalette bind:open={store.palette} {commands} />
<Toasts />
