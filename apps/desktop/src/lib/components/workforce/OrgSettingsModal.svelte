<script lang="ts">
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
  const budgetError = $derived(micros === undefined ? "Write an amount in dollars, such as 120." : null);
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
      toast.ok("Saved.");
      open = false;
      await workforce.refresh();
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      busy = false;
    }
  }
</script>

<Modal bind:open title="Org settings" icon="sliders" size="md">
  <p class="mb-4 text-sm text-muted">What applies to the whole organisation.</p>
  <div class="flex flex-col gap-4">
    <div class="grid grid-cols-2 gap-3">
      <Field label="Budget per week" hint="Across all workers. Empty means no limit." error={budgetError}>
        {#snippet children(f)}<Input id={f.id} bind:value={budget} icon="dollar" inputmode="decimal" placeholder="120" invalid={f.invalid} />{/snippet}
      </Field>
      <Field label="Week starts" hint="Budgets reset then.">
        {#snippet children(f)}<Input id={f.id} value="Monday" disabled />{/snippet}
      </Field>
    </div>
    <Field label="Level for new workers" hint="A worker still cannot be freer than who it reports to.">
      {#snippet children()}<AutonomyDial bind:level label="Level for new workers" />{/snippet}
    </Field>
    <section aria-label="Departments" class="flex flex-col gap-2">
      <h3 class="text-sm font-medium">Departments</h3>
      <ul class="divide-y divide-line-soft rounded-xl border border-line">
        {#each departments as d, i (d.slug)}
          <li class="flex items-center gap-2 px-3 py-2">
            <Input class="flex-1" size="sm" bind:value={d.name} maxlength={60} aria-label="Name of {d.slug}" />
            <Select class="w-36" bind:value={d.lead} aria-label="Lead of {d.name}">
              <option value="">No lead</option>
              {#each workers.filter((w) => w.worker.department === d.slug) as w (w.worker.agent)}<option value={w.worker.agent}>{w.name}</option>{/each}
            </Select>
            <span class="tnum w-16 text-right text-xs text-muted">{used(d.slug)} {used(d.slug) === 1 ? "worker" : "workers"}</span>
            <IconButton icon="trash" label="Remove {d.name}" disabled={used(d.slug) > 0} title={used(d.slug) > 0 ? "It has workers" : undefined} onclick={() => (departments = departments.filter((_, k) => k !== i))} />
          </li>
        {/each}
      </ul>
      <div class="flex items-center gap-2">
        <Input class="flex-1" size="sm" bind:value={pending} placeholder="New department" maxlength={60} aria-label="Name of a new department" onkeydown={(e) => e.key === "Enter" && add()} />
        <Button size="sm" icon="plus" disabled={pending.trim() === ""} onclick={add}>Add department</Button>
      </div>
    </section>
  </div>
  {#snippet footer()}
    <Button disabled={busy} onclick={() => (open = false)}>Cancel</Button>
    <Button variant="primary" icon="check" disabled={!valid || busy} onclick={() => void save()}>Save</Button>
  {/snippet}
</Modal>
