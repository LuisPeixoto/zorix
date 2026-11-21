<script lang="ts">
  import { type Project, errorText, projectCreate, projectRemove, projectUpdate } from "../../api.ts";
  import { isProjectKey, microsToUsd, suggestKey, usdToMicros } from "../../project.ts";
  import { projects } from "../../stores/projects.svelte.ts";
  import { store } from "../../stores/runs.svelte.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { Button, Field, Input, Modal, Select, Textarea } from "../../ui/index.ts";

  /** Makes a project, or (with `project`) changes one: its name, dates, manager and the cost limit of each task run. */
  let { open = $bindable(false), project = null }: { open?: boolean; project?: Project | null } = $props();

  let name = $state("");
  let key = $state("");
  let keyTouched = $state(false);
  let description = $state("");
  let manager = $state("");
  let start = $state("");
  let due = $state("");
  let limit = $state("");
  let busy = $state(false);
  let confirmDelete = $state(false);

  // Each time it opens it starts from the project, or empty.
  $effect(() => {
    if (!open) return;
    name = project?.name ?? "";
    key = project?.key ?? "";
    keyTouched = project !== null;
    description = project?.description ?? "";
    manager = project?.manager ?? "";
    start = project?.start ?? "";
    due = project?.due ?? "";
    limit = microsToUsd(project?.cost_limit_micros ?? null);
    confirmDelete = false;
    void projects.loadPeople();
  });

  $effect(() => {
    if (!project && !keyTouched) key = suggestKey(name);
  });

  const cost = $derived(usdToMicros(limit));
  const keyError = $derived(
    project || key === "" ? null : !isProjectKey(key) ? "Use 2 to 5 capital letters or digits, starting with a letter." : projects.cards.some((c) => c.project.key === key) ? "Another project already uses this key." : null,
  );
  const dateError = $derived(start && due && start > due ? "A project cannot start after it is due." : null);
  const costError = $derived(cost === undefined ? "Write an amount in dollars, such as 1.50." : null);
  const valid = $derived(name.trim() !== "" && (project !== null || (key !== "" && !keyError)) && !dateError && !costError);

  async function save() {
    if (!valid || busy) return;
    busy = true;
    try {
      const fields = { name: name.trim(), description, manager: manager || null, start: start || null, due: due || null, cost_limit_micros: cost ?? null };
      if (project) {
        await projectUpdate(project.key, { ...fields, archived: project.archived });
        toast.ok("Saved.");
        open = false;
        await projects.refreshBoard();
      } else {
        const made = await projectCreate({ key, ...fields });
        toast.ok(`${made.name} is ready. Add its first task.`);
        open = false;
        store.showProjects(made.key);
      }
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      busy = false;
    }
  }

  async function archive() {
    if (!project) return;
    busy = true;
    try {
      await projectUpdate(project.key, {
        name: project.name,
        description: project.description,
        manager: project.manager,
        start: project.start,
        due: project.due,
        cost_limit_micros: project.cost_limit_micros,
        archived: !project.archived,
      });
      toast.ok(project.archived ? `${project.name} is active again.` : `${project.name} is archived.`);
      open = false;
      store.showProjects();
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      busy = false;
    }
  }

  async function remove() {
    if (!project) return;
    busy = true;
    try {
      await projectRemove(project.key);
      toast.ok(`${project.name} was deleted.`);
      open = false;
      store.showProjects();
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      busy = false;
    }
  }
</script>

<Modal bind:open title={project ? "Project settings" : "New project"} icon="kanban" size="md">
  <p class="mb-4 text-sm text-muted">{project ? "Its key cannot change. Tasks keep their keys." : "Its tasks get keys from the project: OPS-1, OPS-2 and so on."}</p>
  <div class="flex flex-col gap-4">
    <div class="grid grid-cols-[1fr_7.5rem] gap-3">
      <Field label="Name" required>
        {#snippet children(f)}<Input id={f.id} bind:value={name} maxlength={200} placeholder="Q4 Operations" />{/snippet}
      </Field>
      <Field label="Key" hint={project ? "Fixed" : "2 to 5 capitals"} error={keyError} required>
        {#snippet children(f)}<Input id={f.id} mono bind:value={key} disabled={project !== null} maxlength={5} invalid={f.invalid} oninput={() => { keyTouched = true; key = key.toUpperCase(); }} />{/snippet}
      </Field>
    </div>
    <Field label="Description">
      {#snippet children(f)}<Textarea id={f.id} rows={3} bind:value={description} maxlength={20000} placeholder="What it is for." />{/snippet}
    </Field>
    <div class="grid grid-cols-2 gap-3">
      <Field label="Start">
        {#snippet children(f)}<Input id={f.id} type="date" bind:value={start} />{/snippet}
      </Field>
      <Field label="Due" error={dateError}>
        {#snippet children(f)}<Input id={f.id} type="date" bind:value={due} invalid={f.invalid} />{/snippet}
      </Field>
    </div>
    <div class="grid grid-cols-2 gap-3">
      <Field label="Manager" hint="An agent that plans the project. It asks before it changes anything.">
        {#snippet children(f)}
          <Select id={f.id} bind:value={manager}>
            <option value="">No manager</option>
            {#each projects.agents.filter((a) => !a.archived) as agent (agent.slug)}<option value={agent.slug}>{agent.name}</option>{/each}
          </Select>
        {/snippet}
      </Field>
      <Field label="Cost limit per task run" hint="Each task run stops at this cost. Empty means no limit." error={costError}>
        {#snippet children(f)}<Input id={f.id} bind:value={limit} icon="dollar" inputmode="decimal" placeholder="1.00" invalid={f.invalid} />{/snippet}
      </Field>
    </div>
  </div>
  {#snippet footer()}
    {#if project}
      {#if confirmDelete}
        <span class="mr-auto text-sm text-err">Delete {project.key} with all its tasks?</span>
        <Button disabled={busy} onclick={() => (confirmDelete = false)}>Keep it</Button>
        <Button variant="danger" icon="trash" disabled={busy} onclick={() => void remove()}>Delete project</Button>
      {:else}
        <span class="mr-auto flex gap-1.5">
          <Button variant="ghost" icon="archive" disabled={busy} onclick={() => void archive()}>{project.archived ? "Bring back" : "Archive"}</Button>
          <Button variant="ghost" icon="trash" disabled={busy} onclick={() => (confirmDelete = true)}>Delete</Button>
        </span>
        <Button disabled={busy} onclick={() => (open = false)}>Cancel</Button>
        <Button variant="primary" icon="check" disabled={!valid || busy} onclick={() => void save()}>Save</Button>
      {/if}
    {:else}
      <Button disabled={busy} onclick={() => (open = false)}>Cancel</Button>
      <Button variant="primary" icon="plus" disabled={!valid || busy} onclick={() => void save()}>Create project</Button>
    {/if}
  {/snippet}
</Modal>
