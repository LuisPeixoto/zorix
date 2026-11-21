<script lang="ts">
  import { type TaskPriority, type TaskState, type TaskView, errorText, taskAdd } from "../../api.ts";
  import { LANE_NAME, PRIORITIES, PRIORITY, whoKey, whoOf } from "../../project.ts";
  import { projects } from "../../stores/projects.svelte.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { Button, Chip, Field, Input, Modal, Segmented, Select, Textarea } from "../../ui/index.ts";

  /** Adds a task to a project, in the lane it was asked for from. */
  let {
    open = $bindable(false),
    project,
    lane = "backlog",
    tasks,
    onadded,
  }: { open?: boolean; project: string; lane?: TaskState; tasks: TaskView[]; onadded: (key: string) => void } = $props();

  let title = $state("");
  let description = $state("");
  let priority = $state<TaskPriority>("normal");
  let assignee = $state("none");
  let start = $state("");
  let due = $state("");
  let after = $state<string[]>([]);
  let pick = $state("");
  let busy = $state(false);

  $effect(() => {
    if (!open) return;
    title = "";
    description = "";
    priority = "normal";
    assignee = "none";
    start = "";
    due = "";
    after = [];
    pick = "";
    void projects.loadPeople();
  });

  const dateError = $derived(start && due && start > due ? "A task cannot start after it is due." : null);
  const valid = $derived(title.trim() !== "" && !dateError);
  const runs = $derived(assignee.startsWith("agent:") || assignee.startsWith("team:"));
  const options = $derived(tasks.filter((t) => !after.includes(t.task.key)));

  async function create() {
    if (!valid || busy) return;
    busy = true;
    try {
      const made = await taskAdd(project, {
        title: title.trim(),
        description,
        state: lane,
        priority,
        assignee: whoOf(assignee),
        start: start || null,
        due: due || null,
        depends_on: after,
      });
      toast.ok(`${made.key} added${lane === "in_progress" && runs ? ". Its run is starting." : "."}`);
      open = false;
      onadded(made.key);
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      busy = false;
    }
  }
</script>

<Modal bind:open title="New task" icon="plus" size="md">
  <p class="mb-4 text-sm text-muted">It starts in {LANE_NAME[lane]}.{runs && lane === "in_progress" ? " Its agent starts working on it as soon as you add it." : ""}</p>
  <div class="flex flex-col gap-4">
    <Field label="Title" required>
      {#snippet children(f)}<Input id={f.id} bind:value={title} maxlength={200} placeholder="Triage the incident backlog" />{/snippet}
    </Field>
    <Field label="Description" hint="What to do and what good looks like. An agent reads this.">
      {#snippet children(f)}<Textarea id={f.id} rows={3} bind:value={description} maxlength={20000} />{/snippet}
    </Field>
    <div class="grid grid-cols-2 gap-3">
      <Field label="Who has it" hint={runs ? "Moving it to In progress starts a run." : undefined}>
        {#snippet children(f)}
          <Select id={f.id} bind:value={assignee}>
            <option value="none">Nobody yet</option>
            <option value={whoKey({ type: "you" })}>You</option>
            {#each projects.agents.filter((a) => !a.archived) as agent (agent.slug)}<option value={`agent:${agent.slug}`}>{agent.name}</option>{/each}
            {#each projects.teams as team (team.team.slug)}<option value={`team:${team.team.slug}`}>{team.team.name} (team)</option>{/each}
          </Select>
        {/snippet}
      </Field>
      <Field label="Priority">
        {#snippet children()}
          <Segmented bind:value={priority} label="Priority" options={PRIORITIES.map((p) => ({ value: p, label: PRIORITY[p].label }))} />
        {/snippet}
      </Field>
    </div>
    <div class="grid grid-cols-2 gap-3">
      <Field label="Start">
        {#snippet children(f)}<Input id={f.id} type="date" bind:value={start} />{/snippet}
      </Field>
      <Field label="Due" error={dateError}>
        {#snippet children(f)}<Input id={f.id} type="date" bind:value={due} invalid={f.invalid} />{/snippet}
      </Field>
    </div>
    {#if tasks.length > 0}
      <Field label="Waits for" hint="Its run starts when these are done.">
        {#snippet children(f)}
          <div class="flex flex-wrap items-center gap-2">
            {#each after as key (key)}<Chip onremove={() => (after = after.filter((k) => k !== key))}>{key}</Chip>{/each}
            <Select id={f.id} bind:value={pick} class="w-56" onchange={() => { if (pick) { after = [...after, pick]; pick = ""; } }}>
              <option value="">Add a task…</option>
              {#each options as t (t.task.key)}<option value={t.task.key}>{t.task.key} · {t.task.title}</option>{/each}
            </Select>
          </div>
        {/snippet}
      </Field>
    {/if}
  </div>
  {#snippet footer()}
    <Button disabled={busy} onclick={() => (open = false)}>Cancel</Button>
    <Button variant="primary" icon="plus" disabled={!valid || busy} onclick={() => void create()}>Add task</Button>
  {/snippet}
</Modal>
