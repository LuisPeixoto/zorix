<script lang="ts">
  import { t } from "../../i18n/index.ts";
  import { errorText, taskAdd } from "../../api.ts";
  import { projects } from "../../stores/projects.svelte.ts";
  import { store } from "../../stores/runs.svelte.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { Button, Field, Input, Modal, Select } from "../../ui/index.ts";

  /** Gives a worker a task: pick the project and say what it is. The task starts in the Backlog; moving it to In progress starts the run. */
  let { open = $bindable(false), agent, name }: { open?: boolean; agent: string; name: string } = $props();

  let project = $state("");
  let title = $state("");
  let due = $state("");
  let busy = $state(false);

  $effect(() => {
    if (!open) return;
    title = "";
    due = "";
    void projects.refreshList().then(() => {
      if (!projects.cards.some((c) => c.project.key === project)) project = projects.cards[0]?.project.key ?? "";
    });
  });

  const choices = $derived(projects.cards.filter((c) => !c.project.archived));
  const valid = $derived(project !== "" && title.trim() !== "");

  async function save() {
    if (!valid || busy) return;
    busy = true;
    try {
      const task = await taskAdd(project, { title: title.trim(), assignee: { type: "agent", slug: agent }, due: due || null, state: "backlog" });
      toast.ok(t("workforce.assignTaskModal.wasGivenTo", { key: task.key, name }));
      open = false;
      store.showProjects(project, "board", task.key);
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      busy = false;
    }
  }
</script>

<Modal bind:open title={t("common.assignATask")} icon="kanban" size="sm">
  <p class="mb-4 text-sm text-muted">{t("workforce.assignTaskModal.getsTheTaskMove", { name })}</p>
  <div class="flex flex-col gap-4">
    <Field label={t("workforce.assignTaskModal.project")} hint={choices.length === 0 ? t("workforce.assignTaskModal.thereIsNoProject") : undefined} required>
      {#snippet children(f)}
        <Select id={f.id} bind:value={project}>
          {#each choices as c (c.project.key)}<option value={c.project.key}>{c.project.key} · {c.project.name}</option>{/each}
        </Select>
      {/snippet}
    </Field>
    <Field label={t("common.task")} required>
      {#snippet children(f)}<Input id={f.id} bind:value={title} maxlength={200} placeholder={t("workforce.assignTaskModal.reviewTheOpenPull")} />{/snippet}
    </Field>
    <Field label={t("common.due")}>
      {#snippet children(f)}<Input id={f.id} type="date" bind:value={due} />{/snippet}
    </Field>
  </div>
  {#snippet footer()}
    <Button disabled={busy} onclick={() => (open = false)}>{t("common.cancel")}</Button>
    <Button variant="primary" icon="plus" disabled={!valid || busy} onclick={() => void save()}>{t("workforce.assignTaskModal.assign")}</Button>
  {/snippet}
</Modal>
