<script lang="ts">
  import { t as tr } from "../../i18n/index.ts";
  import { type TaskState, errorText, projectAsk } from "../../api.ts";
  import { FILTERS, type Filter, type Scale, countOf, dueLabel, filterTasks, progressOf, todayOf, whoKey } from "../../project.ts";
  import { PROJECT_TABS, type ProjectTab, projects } from "../../stores/projects.svelte.ts";
  import { store } from "../../stores/runs.svelte.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { AvatarStack, Badge, Button, Callout, Chip, Field, Icon, IconButton, Modal, Segmented, Skeleton, Sparkbar, Tabs, Textarea } from "../../ui/index.ts";
  import NewProjectModal from "./NewProjectModal.svelte";
  import NewTaskModal from "./NewTaskModal.svelte";
  import ProjectBoard from "./ProjectBoard.svelte";
  import ProjectDeliverables from "./ProjectDeliverables.svelte";
  import ProjectHistory from "./ProjectHistory.svelte";
  import ProjectList from "./ProjectList.svelte";
  import ProjectTimeline from "./ProjectTimeline.svelte";
  import TaskDrawer from "./TaskDrawer.svelte";
  import WhoAvatar from "./WhoAvatar.svelte";

  let { key }: { key: string } = $props();

  const board = $derived(projects.board);
  const project = $derived(board?.project ?? null);
  const views = $derived(board?.tasks ?? []);
  const today = $derived(todayOf(store.now));

  let filter = $state<Filter>("everyone");
  let scale = $state<Scale>("weeks");
  let adding = $state(false);
  let lane = $state<TaskState>("backlog");
  let editing = $state(false);
  let asking = $state(false);
  let prompt = $state("");
  let busy = $state(false);
  let drawer = $state(false);

  // The board is alive: runs finish, tasks move and the manager plans while it is open.
  $effect(() => {
    void projects.loadPeople();
    void projects.refreshBoard();
    const timer = setInterval(() => void projects.refreshBoard(), 2000);
    return () => clearInterval(timer);
  });

  $effect(() => {
    drawer = projects.taskKey !== null;
  });

  const counts = $derived(views.reduce((c, v) => ({ ...c, [v.task.state]: c[v.task.state] + 1 }), { backlog: 0, in_progress: 0, review: 0, done: 0 }));
  const progress = $derived(progressOf(counts));
  const shown = $derived(filterTasks(views, filter));
  const deliverableCount = $derived(views.reduce((n, v) => n + v.task.deliverables.length, 0));
  const people = $derived.by(() => {
    const seen = new Map<string, NonNullable<(typeof views)[number]["task"]["assignee"]>>();
    for (const v of views) if (v.task.assignee) seen.set(whoKey(v.task.assignee), v.task.assignee);
    return [...seen.values()];
  });
  const manager = $derived(project?.manager ?? null);
  const managerName = $derived(manager ? projects.agentName(manager) : "");
  const due = $derived(dueLabel(project?.due ?? null, today));
  const tabs = $derived(PROJECT_TABS.map((t) => ({ value: t, label: t === "board" ? tr("projects.projectPage.board") : t === "list" ? tr("projects.projectPage.list") : t === "timeline" ? tr("projects.projectPage.timeline") : t === "deliverables" ? tr("projects.projectPage.deliverables") : tr("projects.projectPage.history"), count: t === "deliverables" ? deliverableCount : undefined })));

  const open = (taskKey: string) => projects.openTask(taskKey);
  const add = (to: TaskState) => {
    lane = to;
    adding = true;
  };

  async function ask() {
    if (!project) return;
    busy = true;
    try {
      const run = await projectAsk(project.key, prompt.trim() || undefined);
      asking = false;
      prompt = "";
      toast.ok(tr("projects.projectPage.isPlanningEveryChange", { managerName }));
      await projects.refreshBoard();
      void run;
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      busy = false;
    }
  }

  const tab = $derived(projects.tab);
  function setTab(value: string) {
    projects.tab = value as ProjectTab;
  }
</script>

<div class="h-full overflow-y-auto">
  <div class="mx-auto w-full max-w-[1480px] p-6">
    {#if projects.error && !board}
      <Callout tone="err" title={tr("projects.projectPage.thisProjectCouldNot")}>{projects.error}</Callout>
      <div class="mt-4"><Button icon="kanban" onclick={() => store.showProjects()}>{tr("projects.projectPage.allProjects")}</Button></div>
    {:else if !project}
      <div class="flex flex-col gap-3" aria-busy="true" aria-label={tr("projects.projectPage.loadingTheProject")}><Skeleton class="h-8 w-64" /><Skeleton class="h-4 w-96" /><Skeleton class="mt-6 h-72 w-full" /></div>
    {:else}
      <header class="relative mb-4 flex flex-wrap items-end gap-4 border-b border-line-soft pb-4">
        <span class="glow -top-20 -left-16 size-56" aria-hidden="true"></span>
        <div class="relative min-w-60 flex-1">
          <nav aria-label={tr("common.breadcrumb")} class="mb-0.5 flex items-center gap-1.5 text-xs text-muted">
            <button type="button" class="inline-flex items-center gap-1.5 rounded outline-none hover:text-fg focus-visible:underline" onclick={() => store.showProjects()}><Icon name="kanban" size={13} />{tr("common.projects")}</button>
            <Icon name="chevron-right" size={12} />
            <span class="mono">{project.key}</span>
            {#if project.archived}<Badge tone="muted">{tr("projects.projectPage.archived")}</Badge>{/if}
          </nav>
          <h1 class="text-3xl font-semibold tracking-tight">{project.name}</h1>
          <p class="mt-1 text-base text-muted">
            {#if project.description}{project.description}{" "}{/if}{tr("projects.projectPage.ownerYou")}{manager ? tr("projects.projectPage.manager", { managerName }) : ""}{project.due ? tr("projects.projectPage.due", { due: due && !due.late ? due.text : due ? `${due.text} (${project.due})` : project.due }) : ""}.
          </p>
        </div>
        <div class="relative flex w-55 flex-col gap-1.5" aria-label={tr("common.progress")}>
          <span class="flex items-center justify-between text-xs"><span class="text-muted">{tr("common.progress")}</span><b class="tnum font-semibold">{progress.text}</b></span>
          <Sparkbar value={progress.percent} max={100} label={tr("common.progress")} class="w-full" />
        </div>
        <div class="relative flex items-center gap-2">
          <AvatarStack extra={Math.max(0, people.length - 5)}>
            {#each people.slice(0, 5) as who (whoKey(who))}<WhoAvatar {who} />{/each}
          </AvatarStack>
          {#if board?.manager_run}
            <button type="button" class="outline-none" aria-label={tr("projects.projectPage.isPlanningOpenIts", { managerName })} onclick={() => void store.select(board.manager_run?.execution ?? "")}>
              <Badge tone={board.manager_run.waiting ? "warn" : "info"} live>{managerName} {board.manager_run.waiting ? tr("projects.projectPage.waitsForYou") : tr("projects.projectPage.isPlanning")}</Badge>
            </button>
          {:else if manager}
            <Button icon="gauge" disabled={project.archived} onclick={() => (asking = true)}>{tr("projects.projectPage.ask", { managerName })}</Button>
          {/if}
          <IconButton label={tr("projects.projectPage.projectSettings")} icon="sliders" variant="secondary" onclick={() => (editing = true)} />
          <Button variant="primary" icon="plus" disabled={project.archived} onclick={() => add("backlog")}>{tr("projects.projectPage.newTask")}</Button>
        </div>
      </header>

      <div class="mb-4 flex flex-wrap items-end justify-between gap-3">
        <Tabs {tabs} value={tab} label={tr("projects.projectPage.viewsOfTheProject")} class="border-b-0" />
        {#if tab === "board" || tab === "list"}
          <div class="flex flex-wrap items-center gap-2" role="group" aria-label={tr("projects.projectPage.whoseTasks")}>
            {#each FILTERS as f (f.id)}<Chip selected={filter === f.id} onclick={() => (filter = f.id)}>{f.label}</Chip>{/each}
          </div>
        {:else if tab === "timeline"}
          <Segmented bind:value={scale} label={tr("projects.projectPage.scale")} options={[{ value: "days", label: "Days" }, { value: "weeks", label: "Weeks" }, { value: "months", label: "Months" }]} class="w-60" />
        {/if}
      </div>

      {#if tab === "board"}
        {#if countOf(counts) === 0}
          <Callout tone="info" title={tr("projects.projectPage.noTasksYet")}>{tr("projects.projectPage.addTheFirstTask")}</Callout>
        {/if}
        <div class="mt-3"><ProjectBoard views={shown} {manager} {today} onopen={open} onadd={add} /></div>
      {:else if tab === "list"}
        <ProjectList views={shown} {manager} {today} onopen={open} onadd={add} />
      {:else if tab === "timeline"}
        <ProjectTimeline {views} {today} {scale} onopen={open} />
      {:else if tab === "deliverables"}
        <ProjectDeliverables {views} onopen={open} />
      {:else}
        <ProjectHistory project={project.key} onopen={open} />
      {/if}

      {#if projects.taskKey}
        {#key projects.taskKey}<TaskDrawer bind:open={drawer} taskKey={projects.taskKey} {project} {views} onask={() => (asking = true)} />{/key}
      {/if}
      <NewTaskModal bind:open={adding} project={project.key} {lane} tasks={views} onadded={(k) => { void projects.refreshBoard(); open(k); }} />
      <NewProjectModal bind:open={editing} {project} />

      <Modal bind:open={asking} title={tr("projects.projectPage.ask", { managerName })} icon="gauge" size="md">
        <p class="mb-3 text-sm text-muted">{tr("projects.projectPage.readsTheProjectS", { managerName })}</p>
        <Field label={tr("common.whatToAsk")} hint={tr("projects.projectPage.leaveItEmptyAnd")}>
          {#snippet children(f)}<Textarea id={f.id} rows={3} bind:value={prompt} placeholder={tr("projects.projectPage.whatIsLateOr")} maxlength={4000} />{/snippet}
        </Field>
        {#snippet footer()}
          <Button disabled={busy} onclick={() => (asking = false)}>{tr("common.cancel")}</Button>
          <Button variant="primary" icon="send" disabled={busy} onclick={() => void ask()}>{tr("projects.projectPage.ask", { managerName })}</Button>
        {/snippet}
      </Modal>

      <Modal open={projects.pendingStop !== null} onclose={() => (projects.pendingStop = null)} title={tr("projects.projectPage.stopTheRun")} icon="alert" size="sm">
        <p class="text-base text-muted">{tr("projects.projectPage.hasARunGoing", { key: projects.pendingStop?.key })}</p>
        {#snippet footer()}
          <Button onclick={() => (projects.pendingStop = null)}>{tr("projects.projectPage.keepItGoing")}</Button>
          <Button
            variant="danger"
            icon="x"
            onclick={() => {
              const p = projects.pendingStop;
              if (p) void projects.move(p.key, p.to, p.index, p.note, true);
            }}>{tr("projects.projectPage.stopItAndMove")}</Button
          >
        {/snippet}
      </Modal>
    {/if}
  </div>
</div>
