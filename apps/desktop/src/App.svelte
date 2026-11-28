<script lang="ts">
  import { t as tr } from "./lib/i18n/index.ts";
  import { onMount } from "svelte";

  import { store } from "./lib/stores/runs.svelte.ts";
  import { theme } from "./lib/stores/theme.svelte.ts";
  import NewRun from "./lib/components/NewRun.svelte";
  import Inbox from "./lib/components/Inbox.svelte";
  import Documents from "./lib/components/Documents.svelte";
  import Memory from "./lib/components/Memory.svelte";
  import RunsPage from "./lib/components/RunsPage.svelte";
  import Settings from "./lib/components/Settings.svelte";
  import Agents from "./lib/components/agents/Agents.svelte";
  import Projects from "./lib/components/projects/Projects.svelte";
  import Workforce from "./lib/components/workforce/Workforce.svelte";
  import { WORKFORCE_TABS, type WorkforceTab, workforce } from "./lib/stores/workforce.svelte.ts";
  import { PROJECT_TABS, type ProjectTab, projects } from "./lib/stores/projects.svelte.ts";
  import Workflows from "./lib/components/workflows/Workflows.svelte";
  import { workflows } from "./lib/stores/workflows.svelte.ts";
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
  import { appReady, workflowRuns } from "./lib/api.ts";
  import { asks } from "./lib/stores/asks.svelte.ts";

  onMount(() => {
    theme.init();
    void store.init().then(() => {
      void appReady().catch(() => undefined);
      void integrations.init();
      void asks.init();
      void agents.refresh();
      void projects.loadPeople();
      void projects.refreshIndex();
      void workforce.refresh();
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
      if (start === "workflows") {
        // `workflows` is the list; `workflows:SLUG` the editor; `workflows:SLUG/versions` its versions; `workflows:SLUG/run` its newest run.
        const [slug, how] = tab ? tab.split("/") : [];
        void workflows.refresh().then(async () => {
          const card = workflows.overview?.cards.find((c) => c.workflow.slug === slug);
          if (!card) return store.showWorkflows();
          if (how === "run") {
            const runs = await workflowRuns(card.workflow.id, 1).catch(() => []);
            if (runs[0]) return store.showWorkflows("run", card.workflow.id, runs[0].id);
          }
          store.showWorkflows("editor", card.workflow.id, null, how === "versions" || how === "runs" ? how : null);
        });
      }
      if (start === "projects") {
        // `projects` is the list; `projects:KEY` the board; `projects:KEY/list|timeline|deliverables|history` a tab; `projects:KEY/OPS-38` a task.
        const [project, how] = tab ? tab.split("/") : [];
        if (!project) store.showProjects();
        else if (/^[A-Z][A-Z0-9]+-\d+$/.test(how ?? "")) store.showProjects(project, "board", how);
        else store.showProjects(project, PROJECT_TABS.includes(how as ProjectTab) ? (how as ProjectTab) : "board");
      }
      if (start === "workforce") {
        // `workforce` is the chart; `workforce:list|activity` a tab; `workforce:SLUG` selects a worker; `workforce:SLUG/page` opens its page.
        const [what, how] = tab ? tab.split("/") : [];
        if (WORKFORCE_TABS.includes(what as WorkforceTab)) store.showWorkforce(what as WorkforceTab);
        else if (what) store.showWorkforce("chart", what, how === "page");
        else store.showWorkforce();
      }
      if (start === "inbox") store.showInbox();
      if (start === "runs") store.showRuns();
      if (start === "new") store.showForm();
      if (start === "documents") store.showDocuments();
      if (start === "settings") store.showSettings();
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
    { id: "new", label: tr("common.newRun"), group: tr("app.actions"), icon: "plus", keywords: tr("app.startCreate"), run: () => { store.continuing = null; store.showForm(); } },
    { id: "inbox", label: tr("common.inbox"), group: tr("app.goTo"), icon: "bell", keywords: tr("app.approvalsWaitingReviewDecide"), run: () => store.showInbox() },
    { id: "runs", label: tr("common.runs"), group: tr("app.goTo"), icon: "history", keywords: tr("app.historyListAll"), run: () => store.showRuns() },
    { id: "workflows", label: tr("common.workflows"), group: tr("app.goTo"), icon: "workflow", keywords: tr("app.automationScheduleTriggerWebhook"), run: () => store.showWorkflows() },
    ...(workflows.overview?.cards ?? []).map((card) => ({
      id: `workflow-${card.workflow.slug}`,
      label: tr("app.workflow", { name: card.workflow.draft.name }),
      group: tr("common.workflows"),
      icon: "workflow",
      hint: card.workflow.enabled ? "on" : "off",
      keywords: card.workflow.slug,
      run: () => store.showWorkflows("editor", card.workflow.id),
    })),
    { id: "projects", label: tr("common.projects"), group: tr("app.goTo"), icon: "kanban", keywords: tr("app.tasksBoardListTimeline"), run: () => store.showProjects() },
    ...projects.cards.map((card) => ({
      id: `project-${card.project.key}`,
      label: tr("app.project", { name: card.project.name }),
      group: tr("common.projects"),
      icon: "kanban",
      hint: card.project.key,
      keywords: card.project.key,
      run: () => store.showProjects(card.project.key),
    })),
    ...projects.index.slice(0, 200).map((task) => ({
      id: `task-${task.key}`,
      label: `${task.key} ${task.title}`,
      group: tr("common.tasks"),
      icon: "kanban",
      hint: task.state.replaceAll("_", " "),
      keywords: task.project,
      run: () => store.showProjects(task.project, "board", task.key),
    })),
    { id: "workforce", label: tr("common.workforce"), group: tr("app.goTo"), icon: "building", keywords: tr("app.workersOrganisationOrgChart"), run: () => store.showWorkforce() },
    ...(workforce.overview?.workers ?? []).map((v) => ({
      id: `worker-${v.worker.agent}`,
      label: tr("app.worker", { name: v.name }),
      group: tr("common.workforce"),
      icon: "building",
      hint: v.worker.role,
      keywords: `${v.worker.agent} ${v.worker.department}`,
      run: () => store.showWorkforce("chart", v.worker.agent, true),
    })),
    { id: "agents", label: tr("common.agents"), group: tr("app.goTo"), icon: "bot", keywords: tr("app.assistantWorkerSkillsEvals"), run: () => store.showAgents() },
    { id: "teams", label: tr("common.teams"), group: tr("app.goTo"), icon: "users", keywords: tr("app.leadMembersDelegateOrchestrate"), run: () => store.showTeams() },
    ...agents.teams.map((t) => ({
      id: `team-${t.team.slug}`,
      label: tr("app.team", { name: t.team.name }),
      group: tr("common.teams"),
      icon: "users",
      hint: `${t.team.members.length + 1} agents`,
      keywords: `${t.team.slug} ${t.team.lead}`,
      run: () => store.showTeams(t.team.slug),
    })),
    ...agents.cards.map((card) => ({
      id: `agent-${card.slug}`,
      label: tr("app.agent", { name: card.name }),
      group: tr("common.agents"),
      icon: "bot",
      hint: `v${card.version}`,
      keywords: card.slug,
      run: () => store.showAgents(card.slug),
    })),
    { id: "memory", label: tr("common.memory"), group: tr("app.goTo"), icon: "book", keywords: tr("app.factsRememberPreferences"), run: () => store.showMemory() },
    { id: "documents", label: tr("common.documents"), group: tr("app.goTo"), icon: "folder", keywords: tr("app.knowledgeFoldersFilesSearch"), run: () => store.showDocuments() },
    { id: "settings", label: tr("common.settings"), group: tr("app.goTo"), icon: "sliders", keywords: tr("app.appearanceThemeAboutData"), run: () => store.showSettings() },
    { id: "integrations", label: tr("common.integrations"), group: tr("app.goTo"), icon: "plug", keywords: tr("app.mcpServersConnectionsCatalogue"), run: () => store.showIntegrations() },
    { id: "catalogue", label: tr("app.browseTheServerCatalogue"), group: tr("common.integrations"), icon: "grid", keywords: tr("app.mcpAdd"), run: () => store.showIntegrations("catalogue") },
    { id: "connections", label: tr("app.connectionsAndSandbox"), group: tr("common.integrations"), icon: "link", keywords: tr("app.githubGitlabJiraSlack"), run: () => store.showIntegrations("connections") },
    ...integrations.servers.map((server) => ({
      id: `server-${server.name}`,
      label: tr("app.server", { name: server.name }),
      group: tr("common.integrations"),
      icon: "plug",
      hint: server.state.replaceAll("_", " "),
      run: () => { store.showIntegrations("connected"); void integrations.open(server.name); },
    })),
    { id: "providers", label: tr("app.providersKeys"), group: tr("app.goTo"), icon: "key", keywords: tr("app.modelsApiSettings"), run: () => store.showProviders() },
    { id: "theme", label: tr("app.changeTheme"), group: tr("app.actions"), icon: "sun", hint: tr("app.lightDarkSystem"), keywords: tr("app.darkLightAppearance"), run: () => theme.cycle() },
    ...(import.meta.env.DEV ? [{ id: "design", label: tr("app.designSystem"), group: tr("app.goTo"), icon: "layers", run: () => store.showDesign() }] : []),
    ...store.runs.slice(0, 12).map((run) => ({
      id: `run-${run.id}`,
      label: shorten(run.request.prompt, 70),
      group: tr("common.runs"),
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
    if ((event.metaKey || event.ctrlKey) && !event.shiftKey && event.key.toLowerCase() === "n") {
      event.preventDefault();
      store.continuing = null;
      store.showForm();
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
          <h2 class="text-2xl font-semibold text-err">{tr("app.zorixCouldNotStart")}</h2>
          <p class="selectable mt-2">{store.info.error}</p>
        </div>
      {:else if store.view === "design"}
        <DesignSystem />
      {:else if store.view === "agents"}
        <Agents />
      {:else if store.view === "workflows"}
        <Workflows />
      {:else if store.view === "projects"}
        <Projects />
      {:else if store.view === "workforce"}
        <Workforce />
      {:else if store.view === "memory"}
        <Memory />
      {:else if store.view === "documents"}
        <Documents />
      {:else if store.view === "settings"}
        <Settings />
      {:else if store.view === "integrations"}
        <Integrations />
      {:else if store.view === "providers"}
        <Providers />
      {:else if store.view === "inbox"}
        <Inbox />
      {:else if store.view === "new"}
        <NewRun />
      {:else if store.selectedId}
        {#if store.error}<p class="border-b border-line bg-err/10 px-6 py-2 text-err" role="alert">{store.error}</p>{/if}
        <RunView />
      {:else}
        <RunsPage />
      {/if}
    </main>
  </div>
</div>

<CommandPalette bind:open={store.palette} {commands} />
<Toasts />
