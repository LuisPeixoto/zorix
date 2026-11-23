<script lang="ts">
  import { t } from "../../i18n/index.ts";
  import { store } from "../../stores/runs.svelte.ts";
  import { WORKFORCE_TABS, type WorkforceTab, workforce } from "../../stores/workforce.svelte.ts";
  import { Button, Callout, EmptyState, Icon, LogStream, OrgChart, PageHeader, Segmented, Stat } from "../../ui/index.ts";
  import { YOU, orgNodes } from "../../workforce.ts";
  import { usd } from "../../shell.ts";
  import HireWorkerModal from "./HireWorkerModal.svelte";
  import OrgSettingsModal from "./OrgSettingsModal.svelte";
  import WorkerList from "./WorkerList.svelte";
  import WorkerPanel from "./WorkerPanel.svelte";

  let hiring = $state(false);
  let settingsOpen = $state(false);
  const overview = $derived(workforce.overview);
  const nodes = $derived(overview ? orgNodes(overview) : []);
  const empty = $derived(overview !== null && overview.workers.length === 0);
  const tabs = [
    { value: "chart", label: t("workforce.workforcePage.orgChart"), icon: "building" },
    { value: "list", label: t("workforce.workforcePage.list"), icon: "list" },
    { value: "activity", label: t("workforce.workforcePage.activity"), icon: "activity" },
  ];
  let tab = $state<string>(workforce.tab);
  $effect(() => {
    tab = workforce.tab;
  });
  $effect(() => {
    if (WORKFORCE_TABS.includes(tab as WorkforceTab) && tab !== workforce.tab) workforce.tab = tab as WorkforceTab;
  });

  // The screen is alive: runs go, approvals wait and weeks turn while it is open.
  $effect(() => {
    void workforce.loadAgents();
    void workforce.refresh();
    void workforce.refreshActivity();
    const timer = setInterval(() => {
      void workforce.refresh();
      void workforce.refreshActivity();
    }, 4000);
    return () => clearInterval(timer);
  });

  const dueDetail = $derived(overview ? t("workforce.workforcePage.dueThisWeek", { due_this_week: overview.due_this_week }) : "");
  const waitDetail = $derived(overview ? `${overview.waiting_approvals} ${overview.waiting_approvals === 1 ? "approval" : "approvals"} · ${overview.waiting_reviews} ${overview.waiting_reviews === 1 ? "review" : "reviews"}` : "");
  const lines = $derived(workforce.activity.map((l) => ({ id: l.id, at: l.at, level: l.level, source: l.source, text: l.text, run: l.run })));
  const departments = $derived(overview?.settings.departments.length ?? 0);

  function openLine(line: { run?: string | null }) {
    if (line.run) void store.select(line.run);
  }
</script>

<div class="grid h-full min-h-0 {workforce.selected ? 'grid-cols-[minmax(0,1fr)_22.5rem]' : 'grid-cols-1'}">
  <div class="min-w-0 overflow-y-auto">
    <div class="mx-auto flex max-w-[1100px] flex-col gap-4 p-6">
      <PageHeader class="mb-0" title={t("common.workforce")} subtitle={t("workforce.workforcePage.virtualEmployeesAndManagers")} glow>
        {#snippet actions()}
          <Button icon="sliders" onclick={() => (settingsOpen = true)}>{t("workforce.workforcePage.orgSettings")}</Button>
          <Button variant="primary" icon="plus" onclick={() => (hiring = true)}>{t("workforce.workforcePage.hireAWorker")}</Button>
        {/snippet}
      </PageHeader>

      {#if workforce.error && !overview}
        <Callout tone="err" title={t("workforce.workforcePage.theWorkforceCouldNot")}>{workforce.error}</Callout>
      {:else if overview}
        <div class="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
          <Stat icon="users" label={t("workforce.workforcePage.workers")} value={overview.workers.length} detail="{departments} {departments === 1 ? 'department' : 'departments'}" />
          <Stat icon="kanban" label={t("workforce.workforcePage.openTasks")} value={overview.open_tasks} detail={dueDetail} />
          <Stat icon="shield" label={t("workforce.workforcePage.waitingForPeople")} value={overview.waiting_approvals + overview.waiting_reviews} detail={waitDetail} />
          <Stat icon="dollar" label={t("workforce.workforcePage.spendThisWeek")} value={usd(overview.week_micros).replace(/\.00$/, "")} detail={overview.budget_micros !== null ? `of ${usd(overview.budget_micros).replace(/\.00$/, "")} budget` : "no weekly budget set"} />
        </div>

        {#if empty}
          <EmptyState icon="users" title={t("workforce.workforcePage.noWorkersYet")} text={t("workforce.workforcePage.aWorkerIsAn")}>
            {#snippet actions()}<Button variant="primary" icon="plus" onclick={() => (hiring = true)}>{t("workforce.workforcePage.hireAWorker")}</Button>{/snippet}
          </EmptyState>
        {:else}
          <div class="flex flex-wrap items-center justify-between gap-3">
            <Segmented options={tabs} bind:value={tab} label={t("workforce.workforcePage.organisationView")} />
            {#if tab === "chart"}
              <span class="flex items-center gap-4 text-xs text-muted" aria-label={t("workforce.workforcePage.legend")}>
                <span class="flex items-center gap-1.5"><span class="inline-block size-4 rounded-md border border-accent/30 bg-accent/16" aria-hidden="true"></span>agent</span>
                <span class="flex items-center gap-1.5"><span class="inline-block size-4 rounded-full border border-line-strong bg-hover" aria-hidden="true"></span>person</span>
              </span>
            {/if}
          </div>

          {#if tab === "chart"}
            <div class="canvas-dots rounded-2xl border border-dashed border-line bg-panel-2/40 px-4 py-7">
              <OrgChart {nodes} selected={workforce.selected} onselect={(id) => (workforce.selected = id)} />
            </div>
          {:else if tab === "list"}
            <WorkerList {overview} selected={workforce.selected} onselect={(id) => (workforce.selected = id)} />
            <p class="text-xs text-muted">{t("workforce.workforcePage.aManagerSTasks")}</p>
          {:else}
            <LogStream {lines} now={store.now} scope="all workers" empty={t("workforce.workforcePage.nothingYetWorkersShow")} onopen={openLine} />
          {/if}
        {/if}
      {:else}
        <p class="text-muted" role="status"><Icon name="loader" size={14} class="mr-1.5 inline animate-spin" />{t("workforce.workforcePage.readingTheOrganisation")}</p>
      {/if}
    </div>
  </div>

  {#if workforce.selected && overview}
    <WorkerPanel {overview} id={workforce.selected} onclose={() => (workforce.selected = null)} />
  {/if}
</div>

<HireWorkerModal bind:open={hiring} />
<OrgSettingsModal bind:open={settingsOpen} />

<style>
  .canvas-dots {
    background-image: radial-gradient(var(--line) 1px, transparent 1px);
    background-size: 16px 16px;
  }
</style>
