<script lang="ts">
  import {
    type Project,
    type TaskFields,
    type TaskPage,
    type TaskState,
    type TaskView,
    errorText,
    taskComment,
    taskDeliver,
    taskDetail,
    taskLink,
    taskRemove,
    taskRestart,
    taskUpdate,
  } from "../../api.ts";
  import {
    LANES,
    LANE_NAME,
    LANE_TONE,
    PRIORITIES,
    PRIORITY,
    clock,
    dayLabel,
    eventSentence,
    linkLabel,
    todayOf,
    usd,
    whoKey,
    whoOf,
  } from "../../project.ts";
  import { projects } from "../../stores/projects.svelte.ts";
  import { store } from "../../stores/runs.svelte.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { Badge, Button, Callout, Chip, Drawer, Field, Icon, IconButton, Input, Markdown, Modal, Segmented, Select, Stepper, Textarea } from "../../ui/index.ts";
  import WhoAvatar from "./WhoAvatar.svelte";

  /** One task beside its project: where it is, what it needs from you, its fields, its run, what it produced and what happened. */
  let {
    open = $bindable(false),
    taskKey,
    project,
    views,
    onask,
  }: { open?: boolean; taskKey: string; project: Project; views: TaskView[]; onask: () => void } = $props();

  let page = $state<TaskPage | null>(null);
  let fields = $state<TaskFields | null>(null);
  let base = $state("");
  let loadError = $state("");
  let note = $state("");
  let busy = $state(false);
  let pick = $state("");
  let linkUrl = $state("");
  let adding = $state(false);
  let deliverable = $state({ kind: "answer" as "answer" | "link", title: "", body: "" });
  let openDeliverable = $state<string | null>(null);
  let confirmDelete = $state(false);

  const view = $derived(page?.task ?? null);
  const task = $derived(view?.task ?? null);
  const dirty = $derived(fields !== null && JSON.stringify(fields) !== base);
  const assigneeKey = $derived(fields ? whoKey(fields.assignee) : "none");
  const runs = $derived(task?.runs ?? []);
  const lastRun = $derived(runs.length ? runs[runs.length - 1] : null);

  function fieldsOf(t: TaskView["task"]): TaskFields {
    return {
      title: t.title,
      description: t.description,
      priority: t.priority,
      assignee: t.assignee,
      start: t.start,
      due: t.due,
      depends_on: [...t.depends_on],
      links: t.links.map((l) => ({ ...l })),
      progress: t.progress,
    };
  }

  async function load(initial = false) {
    try {
      const fresh = await taskDetail(taskKey);
      page = fresh;
      loadError = "";
      // What a person is typing is never replaced; what is read fresh fills the form only while it is untouched.
      if (initial || !dirty) {
        fields = fieldsOf(fresh.task.task);
        base = JSON.stringify(fields);
      }
    } catch (e) {
      loadError = errorText(e);
    }
  }

  $effect(() => {
    if (!open) return;
    page = null;
    fields = null;
    note = "";
    confirmDelete = false;
    adding = false;
    void projects.loadPeople();
    void load(true);
    const timer = setInterval(() => void load(), 2000);
    return () => clearInterval(timer);
  });

  async function act(work: () => Promise<unknown>, done?: string) {
    busy = true;
    try {
      await work();
      if (done) toast.ok(done);
      await Promise.all([load(true), projects.refreshBoard(), projects.refreshNeeds()]);
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      busy = false;
    }
  }

  const save = () => fields && act(() => taskUpdate(taskKey, fields as TaskFields), "Saved.");
  const discard = () => {
    if (task) {
      fields = fieldsOf(task);
      base = JSON.stringify(fields);
    }
  };

  async function move(to: TaskState, withNote = false) {
    busy = true;
    const ok = await projects.move(taskKey, to, null, withNote && note.trim() ? note.trim() : null);
    busy = false;
    if (ok) {
      note = "";
      await load(true);
    }
  }

  const comment = () =>
    act(async () => {
      await taskComment(taskKey, note);
      note = "";
    }, "Comment added.");

  const run = () => act(() => taskRestart(taskKey, note.trim() || undefined), "Running again.");

  const addLink = () =>
    act(async () => {
      await taskLink(taskKey, linkUrl.trim());
      linkUrl = "";
    });

  const addDeliverable = () =>
    act(async () => {
      await taskDeliver(taskKey, deliverable.kind, deliverable.title.trim(), deliverable.body.trim());
      deliverable = { kind: "answer", title: "", body: "" };
      adding = false;
    }, "Deliverable added.");

  async function remove() {
    busy = true;
    try {
      await taskRemove(taskKey);
      toast.ok(`${taskKey} was deleted.`);
      open = false;
      projects.taskKey = null;
      await projects.refreshBoard();
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      busy = false;
    }
  }

  function close() {
    projects.taskKey = null;
  }

  const agentRuns = $derived(task?.assignee !== null && task?.assignee?.type !== "you");
  const who = $derived(task?.assignee ? projects.face(task.assignee).label : "");
  const options = $derived(views.filter((v) => v.task.key !== taskKey && !(fields?.depends_on ?? []).includes(v.task.key)));
  const pulls = $derived(Object.fromEntries((page?.links ?? []).map((l) => [l.url, l.pull])));
  const dateError = $derived(fields?.start && fields?.due && fields.start > fields.due ? "A task cannot start after it is due." : null);
  const duration = (a: number, b: number | null) => {
    const minutes = Math.max(1, Math.round(((b ?? store.now) - a) / 60_000));
    return minutes < 60 ? `${minutes} min` : `${Math.floor(minutes / 60)} h ${minutes % 60} min`;
  };
  const OUTCOME = { running: ["info", "Running"], waiting: ["warn", "Waiting for you"], finished: ["ok", "Finished"], failed: ["err", "Failed"], cancelled: ["muted", "Stopped"] } as const;
  const when = (at: number) => (todayOf(at) === todayOf(store.now) ? `today ${clock(at)}` : dayLabel(todayOf(at)));
  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      toast.ok("Copied.");
    } catch {
      toast.err("Could not copy.");
    }
  }
</script>

<Drawer bind:open title={task?.title ?? taskKey} subtitle={task ? `${task.key} · ${PRIORITY[task.priority].label} priority` : "Loading…"} icon="kanban" onclose={close}>
  {#if loadError && !task}
    <div class="p-5"><Callout tone="err" title="This task could not be opened">{loadError}</Callout></div>
  {:else if task && view && fields}
    <div class="flex flex-col gap-5 px-5 py-4">
      {#if view.needs_you === "review"}
        <Callout tone="warn" title="Needs you">
          {agentRuns ? `${who} finished its run and moved the task to Review. Check the deliverables, then mark it done or send it back with a note.` : "It is in Review. Mark it done, or send it back with a note."}
        </Callout>
      {:else if view.needs_you === "run_failed"}
        <Callout tone="err" title="The run could not finish">
          {task.run_requested?.error ?? lastRun?.reason ?? "The run failed."}
          <div class="mt-2"><Button size="sm" icon="refresh" disabled={busy} onclick={() => void run()}>Run again</Button></div>
        </Callout>
      {:else if view.needs_you === "run_waiting"}
        <Callout tone="warn" title="The run waits for you">It asked for an approval. Answer it in the Inbox, or open the run.
          {#if lastRun}<div class="mt-2"><Button size="sm" icon="play" onclick={() => void store.select(lastRun.execution)}>Open the run</Button></div>{/if}
        </Callout>
      {:else if view.blocked_by.length > 0 && task.state === "in_progress"}
        <Callout tone="info" title="Waiting for {view.blocked_by.join(', ')}">Its run starts by itself when they are done.</Callout>
      {/if}

      <Stepper steps={LANES.map((l) => ({ label: l.name }))} current={LANES.findIndex((l) => l.id === task.state)} />

      <div class="grid gap-3.5">
        <Field label="Title" required>
          {#snippet children(f)}<Input id={f.id} bind:value={fields!.title} maxlength={200} />{/snippet}
        </Field>
        <Field label="Description" hint="What to do. An agent reads this when it runs.">
          {#snippet children(f)}<Textarea id={f.id} rows={3} bind:value={fields!.description} maxlength={20000} />{/snippet}
        </Field>
        <div class="grid grid-cols-2 gap-3">
          <Field label="Who has it" hint={runs.length > 0 && lastRun && !lastRun.ended_at ? "A run is going: stop it to change this." : undefined}>
            {#snippet children(f)}
              <Select id={f.id} value={assigneeKey} disabled={lastRun !== null && !lastRun.ended_at} onchange={(e) => (fields!.assignee = whoOf((e.currentTarget as HTMLSelectElement).value))}>
                <option value="none">Nobody</option>
                <option value="you">You</option>
                {#each projects.agents.filter((a) => !a.archived) as agent (agent.slug)}<option value={`agent:${agent.slug}`}>{agent.name}</option>{/each}
                {#each projects.teams as team (team.team.slug)}<option value={`team:${team.team.slug}`}>{team.team.name} (team)</option>{/each}
              </Select>
            {/snippet}
          </Field>
          <Field label="Priority">
            {#snippet children(f)}
              <Select id={f.id} bind:value={fields!.priority}>
                {#each PRIORITIES as p (p)}<option value={p}>{PRIORITY[p].label}</option>{/each}
              </Select>
            {/snippet}
          </Field>
          <Field label="Start">
            {#snippet children(f)}<Input id={f.id} type="date" value={fields!.start ?? ""} oninput={(e) => (fields!.start = (e.currentTarget as HTMLInputElement).value || null)} />{/snippet}
          </Field>
          <Field label="Due" error={dateError}>
            {#snippet children(f)}<Input id={f.id} type="date" value={fields!.due ?? ""} invalid={f.invalid} oninput={(e) => (fields!.due = (e.currentTarget as HTMLInputElement).value || null)} />{/snippet}
          </Field>
        </div>
        <Field label="Progress {fields.progress}%" hint={agentRuns ? "An agent reports this as it works." : undefined}>
          {#snippet children(f)}
            <input id={f.id} type="range" min="0" max="100" step="5" bind:value={fields!.progress} class="w-full accent-accent" aria-label="Progress" />
          {/snippet}
        </Field>
        <Field label="Waits for" hint="Its run starts when these are done.">
          {#snippet children(f)}
            <div class="flex flex-wrap items-center gap-2">
              {#each fields!.depends_on as key (key)}
                {@const dep = views.find((v) => v.task.key === key)}
                <Chip onremove={() => (fields!.depends_on = fields!.depends_on.filter((k) => k !== key))}>
                  <span class="mono mr-1.5 text-2xs text-faint">{key}</span>{#if dep}<Badge tone={LANE_TONE[dep.task.state]} class="h-4.5">{LANE_NAME[dep.task.state]}</Badge>{/if}
                </Chip>
              {/each}
              <Select id={f.id} bind:value={pick} class="w-52" onchange={() => { if (pick) { fields!.depends_on = [...fields!.depends_on, pick]; pick = ""; } }}>
                <option value="">Add a task…</option>
                {#each options as v (v.task.key)}<option value={v.task.key}>{v.task.key} · {v.task.title}</option>{/each}
              </Select>
            </div>
          {/snippet}
        </Field>
      </div>

      {#if dirty}
        <div class="flex items-center justify-end gap-2 rounded-xl border border-accent/30 bg-accent-soft px-3 py-2">
          <span class="mr-auto text-sm text-accent">Unsaved changes</span>
          <Button size="sm" disabled={busy} onclick={discard}>Discard</Button>
          <Button size="sm" variant="primary" icon="check" disabled={busy || !!dateError || !fields.title.trim()} onclick={() => void save()}>Save</Button>
        </div>
      {/if}

      <section aria-label="Links" class="flex flex-col gap-2">
        <h3 class="text-2xs font-semibold tracking-wider text-faint uppercase">Links</h3>
        {#if task.links.length > 0}
          <ul class="flex flex-col gap-1.5" role="list">
            {#each task.links as link (link.url)}
              {@const label = linkLabel(link.url, link.title)}
              {@const pull = pulls[link.url]}
              <li class="flex items-center gap-2 rounded-lg border border-line bg-panel px-2.5 py-1.5 text-sm">
                <Icon name={label.kind === "pr" ? "git-branch" : "link"} size={14} class="text-muted" />
                <span class="min-w-0 flex-1 truncate" title={link.url}>{label.text}</span>
                {#if pull}<Badge tone={pull.state === "merged" ? "accent" : pull.state === "open" ? "ok" : "muted"}>{pull.draft ? "Draft" : pull.state}</Badge>{/if}
                <IconButton label="Copy the address" icon="copy" size="sm" onclick={() => void copy(link.url)} />
              </li>
            {/each}
          </ul>
        {/if}
        <form class="flex gap-2" onsubmit={(e) => { e.preventDefault(); if (linkUrl.trim()) void addLink(); }}>
          <Input bind:value={linkUrl} icon="link" placeholder="https://github.com/acme/api/pull/482" aria-label="A link to add" class="flex-1" />
          <Button type="submit" disabled={busy || !linkUrl.trim()}>Add</Button>
        </form>
      </section>

      <section aria-label="Run" class="flex flex-col gap-2">
        <h3 class="text-2xs font-semibold tracking-wider text-faint uppercase">Run</h3>
        {#if lastRun}
          {@const outcome = OUTCOME[lastRun.outcome]}
          <div class="flex items-center gap-3 rounded-xl border border-line bg-panel p-3">
            <span class="grid size-7.5 shrink-0 place-items-center rounded-lg border border-line bg-panel-2 text-muted"><Icon name="play" size={14} /></span>
            <div class="min-w-0 flex-1">
              <p class="truncate font-semibold">{who || "Run"} · run {runs.length}</p>
              <p class="text-xs text-muted">{lastRun.ended_at ? `${OUTCOME[lastRun.outcome][1]} in ${duration(lastRun.started_at, lastRun.ended_at)}` : `Started ${when(lastRun.started_at)}`} · <span class="mono">{usd(lastRun.cost_micros)}</span>{lastRun.note ? ` · asked to: ${lastRun.note}` : ""}</p>
            </div>
            <Badge tone={outcome[0]} live={lastRun.outcome === "running"}>{outcome[1]}</Badge>
            <Button variant="ghost" size="sm" iconRight="arrow-right" onclick={() => void store.select(lastRun.execution)}>Open run</Button>
          </div>
          {#if runs.length > 1}<p class="text-xs text-muted">{runs.length - 1} earlier {runs.length === 2 ? "run" : "runs"}, {usd(task.runs.reduce((sum, r) => sum + r.cost_micros, 0))} in all.</p>{/if}
        {:else}
          <p class="text-sm text-muted">{agentRuns ? (task.state === "in_progress" ? "Its run is about to start." : "Moving the task to In progress starts its run.") : "Nobody runs this task. Give it to an agent or a team, and move it to In progress to start a run."}</p>
        {/if}
      </section>

      <section aria-label="Deliverables" class="flex flex-col gap-2">
        <div class="flex items-center justify-between">
          <h3 class="text-2xs font-semibold tracking-wider text-faint uppercase">Deliverables</h3>
          <Button variant="ghost" size="sm" icon={adding ? "x" : "plus"} onclick={() => (adding = !adding)}>{adding ? "Cancel" : "Add"}</Button>
        </div>
        {#if adding}
          <div class="flex flex-col gap-2 rounded-xl border border-line bg-panel p-3">
            <Segmented bind:value={deliverable.kind} label="Kind" options={[{ value: "answer", label: "Text", icon: "file-text" }, { value: "link", label: "Link", icon: "link" }]} />
            <Input bind:value={deliverable.title} placeholder="A short name" aria-label="Title" />
            {#if deliverable.kind === "answer"}
              <Textarea rows={3} bind:value={deliverable.body} placeholder="The text" aria-label="Text" />
            {:else}
              <Input bind:value={deliverable.body} placeholder="https://…" aria-label="Address" />
            {/if}
            <div class="flex justify-end"><Button variant="primary" size="sm" icon="check" disabled={busy || !deliverable.title.trim() || !deliverable.body.trim()} onclick={() => void addDeliverable()}>Add deliverable</Button></div>
          </div>
        {/if}
        {#if task.deliverables.length === 0 && !adding}
          <p class="text-sm text-muted">Nothing yet. What a run answers lands here.</p>
        {:else}
          <ul class="divide-y divide-line-soft overflow-hidden rounded-xl border border-line bg-panel" role="list">
            {#each [...task.deliverables].reverse() as d (d.id)}
              <li>
                <div class="flex items-center gap-2.5 px-3 py-2">
                  <Icon name={d.kind === "link" ? "link" : "file-text"} size={15} class="shrink-0 text-muted" />
                  <div class="min-w-0 flex-1">
                    <p class="truncate text-base font-medium">{d.title}</p>
                    <p class="text-xs text-muted">{d.kind === "answer" ? "Answer" : "Link"} · {d.by ? projects.face(d.by).label : "ZORIX"} · {when(d.at)}</p>
                  </div>
                  {#if d.kind === "answer"}
                    <Button variant="ghost" size="sm" onclick={() => (openDeliverable = openDeliverable === d.id ? null : d.id)}>{openDeliverable === d.id ? "Hide" : "Open"}</Button>
                  {:else}
                    <IconButton label="Copy the address" icon="copy" size="sm" onclick={() => void copy(d.body)} />
                  {/if}
                </div>
                {#if openDeliverable === d.id && d.kind === "answer"}<div class="border-t border-line-soft bg-panel-2 px-3.5 py-3"><Markdown text={d.body} /></div>{/if}
              </li>
            {/each}
          </ul>
        {/if}
      </section>

      <section aria-label="Activity" class="flex flex-col gap-2">
        <h3 class="text-2xs font-semibold tracking-wider text-faint uppercase">Activity</h3>
        <ul class="flex flex-col gap-2 text-sm" role="list">
          {#each page?.events.slice(0, 8) ?? [] as event (event.seq)}
            <li class="flex items-center gap-2">
              {#if event.by}<WhoAvatar who={event.by} size="xs" />{:else}<span class="grid size-5 shrink-0 place-items-center rounded-md border border-line bg-panel-2 text-muted"><Icon name="activity" size={11} /></span>{/if}
              <span class="min-w-0 flex-1 truncate" title={eventSentence(event, projects.agentName)}>{eventSentence(event, projects.agentName)}</span>
              <span class="shrink-0 text-xs text-faint">{when(event.at)}</span>
            </li>
          {/each}
        </ul>
        <Field label="Note" hint="A comment, or what to change when you send the task back.">
          {#snippet children(f)}<Textarea id={f.id} rows={2} bind:value={note} maxlength={4000} />{/snippet}
        </Field>
        <div class="flex justify-end"><Button size="sm" icon="message" disabled={busy || !note.trim()} onclick={() => void comment()}>Comment</Button></div>
      </section>

      <div class="border-t border-line-soft pt-3">
        <Button variant="ghost" size="sm" icon="trash" disabled={busy} onclick={() => (confirmDelete = true)}>Delete this task</Button>
      </div>
    </div>
  {/if}

  {#snippet footer()}
    {#if task}
      {#if project.manager}
        <Button variant="ghost" icon="gauge" class="mr-auto" disabled={busy} onclick={onask}>Ask {projects.agentName(project.manager)}</Button>
      {/if}
      {#if task.state === "backlog"}
        <Button variant="primary" icon="play" disabled={busy} onclick={() => void move("in_progress")}>Start</Button>
      {:else if task.state === "in_progress"}
        <Button disabled={busy} onclick={() => void move("review")}>Send to review</Button>
        <Button variant="primary" icon="check" disabled={busy} onclick={() => void move("done")}>Mark done</Button>
      {:else if task.state === "review"}
        <Button icon="undo" disabled={busy} onclick={() => void move("in_progress", true)}>Send back</Button>
        <Button variant="primary" icon="check" disabled={busy} onclick={() => void move("done")}>Mark done</Button>
      {:else}
        <Button icon="undo" disabled={busy} onclick={() => void move("in_progress", true)}>Reopen</Button>
      {/if}
    {/if}
  {/snippet}
</Drawer>

<Modal bind:open={confirmDelete} title="Delete {taskKey}?" icon="trash" size="sm">
  <p class="text-base text-muted">Its deliverables and history go with it. A run that is going is stopped. This cannot be undone.</p>
  {#snippet footer()}
    <Button onclick={() => (confirmDelete = false)}>Keep it</Button>
    <Button variant="danger" icon="trash" disabled={busy} onclick={() => { confirmDelete = false; void remove(); }}>Delete task</Button>
  {/snippet}
</Modal>
