<script lang="ts">
  import { t } from "../../i18n/index.ts";
  import { type AutonomyLevel, errorText, workforceSettingsSave } from "../../api.ts";
  import { usdToMicros } from "../../project.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { workforce } from "../../stores/workforce.svelte.ts";
  import { AutonomyDial, Button, Field, IconButton, Input, Modal, Select } from "../../ui/index.ts";

  /** What applies to the whole organisation: the weekly budget, the level new workers start at, and the departments. */
  let { open = $bindable(false) }: { open?: boolean } = $props();

  let budget = $state("");
  let level = $state<AutonomyLevel>("ask");
  type Row = { slug: string; name: string; description: string; lead: string };
  let departments = $state<Row[]>([]);
  let busy = $state(false);

  const workers = $derived(workforce.overview?.workers ?? []);
  const micros = $derived(usdToMicros(budget));
  const budgetError = $derived(micros === undefined ? t("workforce.orgSettingsModal.writeAnAmountIn") : null);
  const used = (slug: string) => workers.filter((w) => w.worker.department === slug).length;
  const valid = $derived(!budgetError && departments.every((d) => d.name.trim() !== ""));

  $effect(() => {
    if (!open) return;
    const s = workforce.overview?.settings;
    budget = s?.weekly_budget_micros ? String(s.weekly_budget_micros / 1_000_000) : "";
    level = s?.default_level ?? "ask";
    departments = (s?.departments ?? []).map((d) => ({ ...d, lead: d.lead ?? "" }));
  });

  function slugOf(name: string): string {
    const base = name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "") || "department";
    let slug = base;
    for (let n = 2; departments.some((d) => d.slug === slug); n++) slug = `${base}-${n}`;
    return slug;
  }

  let pending = $state("");
  function add() {
    const name = pending.trim();
    if (!name) return;
    departments = [...departments, { slug: slugOf(name), name, description: "", lead: "" }];
    pending = "";
  }

  async function save() {
    if (!valid || busy) return;
    busy = true;
    try {
      await workforceSettingsSave({ weekly_budget_micros: micros ?? null, default_level: level, departments: departments.map((d) => ({ ...d, lead: d.lead || null })) });
      toast.ok(t("common.saved"));
      open = false;
      await workforce.refresh();
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      busy = false;
    }
  }
</script>

<Modal bind:open title={t("workforce.orgSettingsModal.orgSettings")} icon="sliders" size="md">
  <p class="mb-4 text-sm text-muted">{t("workforce.orgSettingsModal.whatAppliesToThe")}</p>
  <div class="flex flex-col gap-4">
    <div class="grid grid-cols-2 gap-3">
      <Field label={t("common.budgetPerWeek")} hint={t("workforce.orgSettingsModal.acrossAllWorkersEmpty")} error={budgetError}>
        {#snippet children(f)}<Input id={f.id} bind:value={budget} icon="dollar" inputmode="decimal" placeholder="120" invalid={f.invalid} />{/snippet}
      </Field>
      <Field label={t("workforce.orgSettingsModal.weekStarts")} hint={t("workforce.orgSettingsModal.budgetsResetThen")}>
        {#snippet children(f)}<Input id={f.id} value="Monday" disabled />{/snippet}
      </Field>
    </div>
    <Field label={t("workforce.orgSettingsModal.levelForNewWorkers")} hint={t("workforce.orgSettingsModal.aWorkerStillCannot")}>
      {#snippet children()}<AutonomyDial bind:level label={t("workforce.orgSettingsModal.levelForNewWorkers")} />{/snippet}
    </Field>
    <section aria-label={t("workforce.orgSettingsModal.departments")} class="flex flex-col gap-2">
      <h3 class="text-sm font-medium">{t("workforce.orgSettingsModal.departments")}</h3>
      <ul class="divide-y divide-line-soft rounded-xl border border-line">
        {#each departments as d, i (d.slug)}
          <li class="flex items-center gap-2 px-3 py-2">
            <Input class="flex-1" size="sm" bind:value={d.name} maxlength={60} aria-label={t("workforce.orgSettingsModal.nameOf", { slug: d.slug })} />
            <Select class="w-36" bind:value={d.lead} aria-label={t("workforce.orgSettingsModal.leadOf", { name: d.name })}>
              <option value="">{t("workforce.orgSettingsModal.noLead")}</option>
              {#each workers.filter((w) => w.worker.department === d.slug) as w (w.worker.agent)}<option value={w.worker.agent}>{w.name}</option>{/each}
            </Select>
            <span class="tnum w-16 text-right text-xs text-muted">{used(d.slug)} {used(d.slug) === 1 ? "worker" : "workers"}</span>
            <IconButton icon="trash" label={t("workforce.orgSettingsModal.remove", { name: d.name })} disabled={used(d.slug) > 0} title={used(d.slug) > 0 ? t("workforce.orgSettingsModal.itHasWorkers") : undefined} onclick={() => (departments = departments.filter((_, k) => k !== i))} />
          </li>
        {/each}
      </ul>
      <div class="flex items-center gap-2">
        <Input class="flex-1" size="sm" bind:value={pending} placeholder={t("workforce.orgSettingsModal.newDepartment")} maxlength={60} aria-label={t("workforce.orgSettingsModal.nameOfANew")} onkeydown={(e) => e.key === "Enter" && add()} />
        <Button size="sm" icon="plus" disabled={pending.trim() === ""} onclick={add}>{t("workforce.orgSettingsModal.addDepartment")}</Button>
      </div>
    </section>
  </div>
  {#snippet footer()}
    <Button disabled={busy} onclick={() => (open = false)}>{t("common.cancel")}</Button>
    <Button variant="primary" icon="check" disabled={!valid || busy} onclick={() => void save()}>{t("common.save")}</Button>
  {/snippet}
</Modal>
