<script lang="ts">
  import { t } from "../../i18n/index.ts";
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
    project || key === "" ? null : !isProjectKey(key) ? t("projects.newProjectModal.use2To5") : projects.cards.some((c) => c.project.key === key) ? t("projects.newProjectModal.anotherProjectAlreadyUses") : null,
  );
  const dateError = $derived(start && due && start > due ? t("projects.newProjectModal.aProjectCannotStart") : null);
  const costError = $derived(cost === undefined ? t("projects.newProjectModal.writeAnAmountIn") : null);
  const valid = $derived(name.trim() !== "" && (project !== null || (key !== "" && !keyError)) && !dateError && !costError);

  async function save() {
    if (!valid || busy) return;
    busy = true;
    try {
      const fields = { name: name.trim(), description, manager: manager || null, start: start || null, due: due || null, cost_limit_micros: cost ?? null };
      if (project) {
        await projectUpdate(project.key, { ...fields, archived: project.archived });
        toast.ok(t("common.saved"));
        open = false;
        await projects.refreshBoard();
      } else {
        const made = await projectCreate({ key, ...fields });
        toast.ok(t("projects.newProjectModal.isReadyAddIts", { name: made.name }));
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
      toast.ok(project.archived ? t("projects.newProjectModal.isActiveAgain", { name: project.name }) : t("projects.newProjectModal.isArchived", { name: project.name }));
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
      toast.ok(t("projects.newProjectModal.wasDeleted", { name: project.name }));
      open = false;
      store.showProjects();
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      busy = false;
    }
  }
</script>

<Modal bind:open title={project ? t("projects.newProjectModal.projectSettings") : t("projects.newProjectModal.newProject")} icon="kanban" size="md">
  <p class="mb-4 text-sm text-muted">{project ? t("projects.newProjectModal.itsKeyCannotChange") : t("projects.newProjectModal.itsTasksGetKeys")}</p>
  <div class="flex flex-col gap-4">
    <div class="grid grid-cols-[1fr_7.5rem] gap-3">
      <Field label={t("common.name")} required>
        {#snippet children(f)}<Input id={f.id} bind:value={name} maxlength={200} placeholder={t("projects.newProjectModal.q4Operations")} />{/snippet}
      </Field>
      <Field label={t("projects.newProjectModal.key")} hint={project ? t("projects.newProjectModal.fixed") : t("projects.newProjectModal.2To5Capitals")} error={keyError} required>
        {#snippet children(f)}<Input id={f.id} mono bind:value={key} disabled={project !== null} maxlength={5} invalid={f.invalid} oninput={() => { keyTouched = true; key = key.toUpperCase(); }} />{/snippet}
      </Field>
    </div>
    <Field label={t("common.description")}>
      {#snippet children(f)}<Textarea id={f.id} rows={3} bind:value={description} maxlength={20000} placeholder={t("projects.newProjectModal.whatItIsFor")} />{/snippet}
    </Field>
    <div class="grid grid-cols-2 gap-3">
      <Field label={t("common.start")}>
        {#snippet children(f)}<Input id={f.id} type="date" bind:value={start} />{/snippet}
      </Field>
      <Field label={t("common.due")} error={dateError}>
        {#snippet children(f)}<Input id={f.id} type="date" bind:value={due} invalid={f.invalid} />{/snippet}
      </Field>
    </div>
    <div class="grid grid-cols-2 gap-3">
      <Field label={t("projects.newProjectModal.manager")} hint={t("projects.newProjectModal.anAgentThatPlans")}>
        {#snippet children(f)}
          <Select id={f.id} bind:value={manager}>
            <option value="">{t("projects.newProjectModal.noManager")}</option>
            {#each projects.agents.filter((a) => !a.archived) as agent (agent.slug)}<option value={agent.slug}>{agent.name}</option>{/each}
          </Select>
        {/snippet}
      </Field>
      <Field label={t("projects.newProjectModal.costLimitPerTask")} hint={t("projects.newProjectModal.eachTaskRunStops")} error={costError}>
        {#snippet children(f)}<Input id={f.id} bind:value={limit} icon="dollar" inputmode="decimal" placeholder="1.00" invalid={f.invalid} />{/snippet}
      </Field>
    </div>
  </div>
  {#snippet footer()}
    {#if project}
      {#if confirmDelete}
        <span class="mr-auto text-sm text-err">{t("projects.newProjectModal.deleteWithAllIts", { key: project.key })}</span>
        <Button disabled={busy} onclick={() => (confirmDelete = false)}>{t("common.keepIt")}</Button>
        <Button variant="danger" icon="trash" disabled={busy} onclick={() => void remove()}>{t("projects.newProjectModal.deleteProject")}</Button>
      {:else}
        <span class="mr-auto flex gap-1.5">
          <Button variant="ghost" icon="archive" disabled={busy} onclick={() => void archive()}>{project.archived ? t("projects.newProjectModal.bringBack") : t("projects.newProjectModal.archive")}</Button>
          <Button variant="ghost" icon="trash" disabled={busy} onclick={() => (confirmDelete = true)}>{t("common.delete")}</Button>
        </span>
        <Button disabled={busy} onclick={() => (open = false)}>{t("common.cancel")}</Button>
        <Button variant="primary" icon="check" disabled={!valid || busy} onclick={() => void save()}>{t("common.save")}</Button>
      {/if}
    {:else}
      <Button disabled={busy} onclick={() => (open = false)}>{t("common.cancel")}</Button>
      <Button variant="primary" icon="plus" disabled={!valid || busy} onclick={() => void save()}>{t("projects.newProjectModal.createProject")}</Button>
    {/if}
  {/snippet}
</Modal>
