<script lang="ts">
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
  const tabs = $derived(PROJECT_TABS.map((t) => ({ value: t, label: t === "board" ? "Board" : t === "list" ? "List" : t === "timeline" ? "Timeline" : t === "deliverables" ? "Deliverables" : "History", count: t === "deliverables" ? deliverableCount : undefined })));

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
      toast.ok(`${managerName} is planning. Every change it asks for waits in your Inbox.`);
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
      <Callout tone="err" title="This project could not be opened">{projects.error}</Callout>
      <div class="mt-4"><Button icon="kanban" onclick={() => store.showProjects()}>All projects</Button></div>
    {:else if !project}
      <div class="flex flex-col gap-3" aria-busy="true" aria-label="Loading the project"><Skeleton class="h-8 w-64" /><Skeleton class="h-4 w-96" /><Skeleton class="mt-6 h-72 w-full" /></div>
    {:else}
      <header class="relative mb-4 flex flex-wrap items-end gap-4 border-b border-line-soft pb-4">
        <span class="glow -top-20 -left-16 size-56" aria-hidden="true"></span>
        <div class="relative min-w-60 flex-1">
          <nav aria-label="Breadcrumb" class="mb-0.5 flex items-center gap-1.5 text-xs text-muted">
            <button type="button" class="inline-flex items-center gap-1.5 rounded outline-none hover:text-fg focus-visible:underline" onclick={() => store.showProjects()}><Icon name="kanban" size={13} />Projects</button>
            <Icon name="chevron-right" size={12} />
            <span class="mono">{project.key}</span>
            {#if project.archived}<Badge tone="muted">Archived</Badge>{/if}
          </nav>
          <h1 class="text-3xl font-semibold tracking-tight">{project.name}</h1>
          <p class="mt-1 text-base text-muted">
            {#if project.description}{project.description}{" "}{/if}Owner: you{manager ? ` · manager: ${managerName}` : ""}{project.due ? ` · due ${due && !due.late ? due.text : due ? `${due.text} (${project.due})` : project.due}` : ""}.
          </p>
        </div>
        <div class="relative flex w-55 flex-col gap-1.5" aria-label="Progress">
          <span class="flex items-center justify-between text-xs"><span class="text-muted">Progress</span><b class="tnum font-semibold">{progress.text}</b></span>
          <Sparkbar value={progress.percent} max={100} label="Progress" class="w-full" />
        </div>
        <div class="relative flex items-center gap-2">
          <AvatarStack extra={Math.max(0, people.length - 5)}>
            {#each people.slice(0, 5) as who (whoKey(who))}<WhoAvatar {who} />{/each}
          </AvatarStack>
          {#if board?.manager_run}
            <button type="button" class="outline-none" aria-label="{managerName} is planning. Open its run." onclick={() => void store.select(board.manager_run?.execution ?? "")}>
              <Badge tone={board.manager_run.waiting ? "warn" : "info"} live>{managerName} {board.manager_run.waiting ? "waits for you" : "is planning"}</Badge>
            </button>
          {:else if manager}
            <Button icon="gauge" disabled={project.archived} onclick={() => (asking = true)}>Ask {managerName}</Button>
          {/if}
          <IconButton label="Project settings" icon="sliders" variant="secondary" onclick={() => (editing = true)} />
          <Button variant="primary" icon="plus" disabled={project.archived} onclick={() => add("backlog")}>New task</Button>
        </div>
      </header>

      <div class="mb-4 flex flex-wrap items-end justify-between gap-3">
        <Tabs {tabs} value={tab} label="Views of the project" class="border-b-0" />
        {#if tab === "board" || tab === "list"}
          <div class="flex flex-wrap items-center gap-2" role="group" aria-label="Whose tasks">
            {#each FILTERS as f (f.id)}<Chip selected={filter === f.id} onclick={() => (filter = f.id)}>{f.label}</Chip>{/each}
          </div>
        {:else if tab === "timeline"}
          <Segmented bind:value={scale} label="Scale" options={[{ value: "days", label: "Days" }, { value: "weeks", label: "Weeks" }, { value: "months", label: "Months" }]} class="w-60" />
        {/if}
      </div>

      {#if tab === "board"}
        {#if countOf(counts) === 0}
          <Callout tone="info" title="No tasks yet">Add the first task. Give it to an agent and move it to In progress to start its run.</Callout>
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

      <Modal bind:open={asking} title="Ask {managerName}" icon="gauge" size="md">
        <p class="mb-3 text-sm text-muted">{managerName} reads the project's tasks and plans. Every change it wants to make waits for you in the Inbox, and a move is approved by you looking at it.</p>
        <Field label="What to ask" hint="Leave it empty and it plans the whole project.">
          {#snippet children(f)}<Textarea id={f.id} rows={3} bind:value={prompt} placeholder="What is late or blocked, and who should have what?" maxlength={4000} />{/snippet}
        </Field>
        {#snippet footer()}
          <Button disabled={busy} onclick={() => (asking = false)}>Cancel</Button>
          <Button variant="primary" icon="send" disabled={busy} onclick={() => void ask()}>Ask {managerName}</Button>
        {/snippet}
      </Modal>

      <Modal open={projects.pendingStop !== null} onclose={() => (projects.pendingStop = null)} title="Stop the run?" icon="alert" size="sm">
        <p class="text-base text-muted">{projects.pendingStop?.key} has a run going. Moving it stops the run, and what it has not finished is lost. What it already produced stays.</p>
        {#snippet footer()}
          <Button onclick={() => (projects.pendingStop = null)}>Keep it going</Button>
          <Button
            variant="danger"
            icon="x"
            onclick={() => {
              const p = projects.pendingStop;
              if (p) void projects.move(p.key, p.to, p.index, p.note, true);
            }}>Stop it and move</Button
          >
        {/snippet}
      </Modal>
    {/if}
  </div>
</div>
