<script lang="ts">
  import { t } from "../../i18n/index.ts";
  import { agents } from "../../stores/agents.svelte.ts";
  import { inbox } from "../../stores/inbox.svelte.ts";
  import { integrations } from "../../stores/integrations.svelte.ts";
  import { store } from "../../stores/runs.svelte.ts";
  import { workforce } from "../../stores/workforce.svelte.ts";
  import { runningCount, spendToday, usd } from "../../shell.ts";
  import { Button, Dot, Kbd } from "../../ui/index.ts";
  import NavItem from "./NavItem.svelte";

  // Workers with a run going: the runs that work, by the agent that runs them, among the agents that are workers.
  const busy = $derived.by(() => {
    const workers = new Set((workforce.overview?.workers ?? []).map((w) => w.worker.agent));
    const working = new Set(store.runs.filter((r) => ["queued", "running", "waiting_for_model", "waiting_for_tool", "waiting_for_children"].includes(r.state) && r.agent && workers.has(r.agent.slug)).map((r) => r.agent?.slug));
    return working.size;
  });
  const waiting = $derived(store.runs.filter((r) => r.state === "waiting_for_approval" || store.needsYou[r.id]).length);
  const running = $derived(runningCount(store.runs));
  const spend = $derived(spendToday(store.runs, store.now));
  const shortcut = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform || navigator.userAgent) ? "⌘N" : t("shell.sidebar.ctrlN");
  const teamsOpen = $derived(store.view === "agents" && agents.section === "teams");
  const agentsOpen = $derived(store.view === "agents" && agents.section === "agents");
</script>

<aside class="flex w-60 shrink-0 flex-col border-r border-line bg-panel-2">
  <div class="p-3">
    <Button variant="subtle" icon="plus" class="w-full justify-start" onclick={() => { store.continuing = null; store.showForm(); }}>
      <span class="flex-1 text-left">{t("common.newRun")}</span><Kbd>{shortcut}</Kbd>
    </Button>
  </div>

  <nav class="min-h-0 flex-1 space-y-4 overflow-y-auto px-3 pb-3" aria-label={t("shell.sidebar.main")}>
    <div>
      <p class="px-2.5 pb-1.5 text-2xs font-semibold tracking-wider text-faint uppercase">{t("shell.sidebar.work")}</p>
      <NavItem icon="bell" active={store.view === "inbox"} count={inbox.count} attention onclick={() => store.showInbox()}>{t("common.inbox")}</NavItem>
      <NavItem icon="history" active={store.view === "runs"} onclick={() => store.showRuns()}>{t("common.runs")}</NavItem>
      <NavItem icon="workflow" active={store.view === "workflows"} onclick={() => store.showWorkflows()}>{t("common.workflows")}</NavItem>
      <NavItem icon="kanban" active={store.view === "projects"} onclick={() => store.showProjects()}>{t("common.projects")}</NavItem>
    </div>
    <div>
      <p class="px-2.5 pb-1.5 text-2xs font-semibold tracking-wider text-faint uppercase">{t("shell.sidebar.organisation")}</p>
      <NavItem icon="bot" active={agentsOpen} onclick={() => store.showAgents()}>{t("common.agents")}</NavItem>
      <NavItem icon="users" active={teamsOpen} onclick={() => store.showTeams()}>{t("common.teams")}</NavItem>
      <NavItem icon="building" active={store.view === "workforce"} onclick={() => store.showWorkforce()}>{t("common.workforce")}</NavItem>
    </div>
    <div>
      <p class="px-2.5 pb-1.5 text-2xs font-semibold tracking-wider text-faint uppercase">{t("shell.sidebar.connect")}</p>
      <NavItem icon="key" active={store.view === "providers"} onclick={() => store.showProviders()}>{t("shell.sidebar.providers")}</NavItem>
      <NavItem icon="plug" active={store.view === "integrations"} count={integrations.attention} attention={integrations.attention > 0} onclick={() => store.showIntegrations()}>{t("common.integrations")}</NavItem>
      <NavItem icon="book" active={store.view === "memory"} onclick={() => store.showMemory()}>{t("common.memory")}</NavItem>
      <NavItem icon="folder" active={store.view === "documents"} onclick={() => store.showDocuments()}>{t("common.documents")}</NavItem>
    </div>
    <div>
      <p class="px-2.5 pb-1.5 text-2xs font-semibold tracking-wider text-faint uppercase">{t("shell.sidebar.operate")}</p>
      {#if import.meta.env.DEV}
        <NavItem icon="layers" active={store.view === "design"} onclick={() => store.showDesign()}>{t("shell.sidebar.designSystem")}</NavItem>
      {/if}
      <NavItem icon="sliders" active={store.view === "settings"} onclick={() => store.showSettings()}>{t("common.settings")}</NavItem>
    </div>
  </nav>

  <footer class="p-3">
    <div class="rounded-2xl border border-line bg-panel px-3.5 py-3 shadow-card">
      <p class="flex items-baseline justify-between text-sm text-muted"><span>{t("shell.sidebar.spendToday")}</span><span class="mono tnum text-base font-semibold text-fg">{usd(spend)}</span></p>
      <p class="mt-2 flex items-center gap-1.5 text-xs text-muted">
        <Dot tone={running > 0 ? "ok" : "muted"} live={running > 0} />
        <span class="tnum">{running} running{busy > 0 ? ` · ${busy} ${busy === 1 ? "worker" : "workers"} busy` : ""}{waiting > 0 ? t("shell.sidebar.waitingForYou", { waiting }) : ""}</span>
      </p>
    </div>
  </footer>
</aside>
