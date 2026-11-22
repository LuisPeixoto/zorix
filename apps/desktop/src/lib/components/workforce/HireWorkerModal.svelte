<script lang="ts">
  import { type AutonomyLevel, type WorkerView, errorText, workerEdit, workerHire } from "../../api.ts";
  import { usdToMicros } from "../../project.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { workforce } from "../../stores/workforce.svelte.ts";
  import { AutonomyDial, Button, Checkbox, Field, Input, Modal, Select } from "../../ui/index.ts";
  import { levelIndex, levelsUnder, money } from "../../workforce.ts";

  /** Makes an agent a worker, or (with `worker`) changes where it is in the organisation. */
  let { open = $bindable(false), worker = null }: { open?: boolean; worker?: WorkerView | null } = $props();

  let agent = $state("");
  let role = $state("");
  let department = $state("");
  let reportsTo = $state("");
  let manager = $state(false);
  let level = $state<AutonomyLevel>("ask");
  let budget = $state("");
  let hours = $state("2");
  let busy = $state(false);

  const overview = $derived(workforce.overview);
  const workers = $derived(overview?.workers ?? []);
  // An agent that is not a worker yet, and not archived.
  const free = $derived(workforce.agents.filter((a) => !a.archived && !workers.some((w) => w.worker.agent === a.slug)));
  // Whoever it could report to: not itself, and nobody who answers to it (that would be a loop).
  const below = $derived.by(() => {
    const out = new Set<string>();
    if (!worker) return out;
    const walk = (slug: string) => {
      for (const w of workers.filter((x) => x.worker.reports_to === slug)) {
        if (out.has(w.worker.agent)) continue;
        out.add(w.worker.agent);
        walk(w.worker.agent);
      }
    };
    walk(worker.worker.agent);
    return out;
  });
  const bosses = $derived(workers.filter((w) => w.worker.agent !== worker?.worker.agent && !below.has(w.worker.agent)));
  const boss = $derived(workers.find((w) => w.worker.agent === reportsTo) ?? null);
  const top = $derived(levelsUnder(boss?.worker.autonomy ?? null).at(-1) ?? "full");
  const cap = $derived(boss?.worker.weekly_budget_micros ?? null);
  const money_ = $derived(usdToMicros(budget));
  const budgetError = $derived(
    money_ === undefined ? "Write an amount in dollars, such as 25." : cap !== null && money_ === null ? `${boss?.name} has a weekly budget, so give this one a budget too, up to ${money(cap)}.` : cap !== null && money_ !== null && money_ > cap ? `Up to ${money(cap)}, the budget of ${boss?.name}.` : null,
  );
  const time = $derived(Number(hours));
  const timeError = $derived(!Number.isFinite(time) || time < 1 / 60 || time > 24 ? "Between 1 minute and 24 hours." : null);
  const valid = $derived((worker !== null || agent !== "") && role.trim() !== "" && department !== "" && !budgetError && !timeError);

  $effect(() => {
    if (!open) return;
    void workforce.loadAgents();
    const w = worker?.worker;
    agent = w?.agent ?? "";
    role = w?.role ?? "";
    department = w?.department ?? overview?.settings.departments[0]?.slug ?? "";
    reportsTo = w?.reports_to ?? "";
    manager = w?.manager ?? false;
    level = w?.autonomy ?? overview?.settings.default_level ?? "ask";
    budget = w?.weekly_budget_micros ? String(w.weekly_budget_micros / 1_000_000) : "";
    hours = w ? String(w.task_time_ms / 3_600_000) : "2";
  });

  // A worker cannot be freer than who it reports to, so the level follows when the manager is chosen.
  $effect(() => {
    if (levelIndex(level) > levelIndex(top)) level = top;
  });

  async function save() {
    if (!valid || busy) return;
    busy = true;
    try {
      const task_time_ms = Math.round(time * 3_600_000);
      if (worker) {
        await workerEdit(worker.worker.agent, {
          role: role.trim(),
          department,
          reports_to: reportsTo || null,
          manager,
          autonomy: level,
          weekly_budget_micros: money_ ?? null,
          task_time_ms,
          still_ask: worker.worker.still_ask,
          schedule_enabled: worker.worker.schedule.enabled,
        });
        toast.ok("Saved.");
      } else {
        await workerHire({ agent, role: role.trim(), department, reports_to: reportsTo || null, manager, autonomy: level, weekly_budget_micros: money_ ?? null, task_time_ms, still_ask: [] });
        toast.ok(`${workforce.agentOf(agent)?.name ?? agent} was hired.`);
      }
      open = false;
      await Promise.all([workforce.refresh(), workforce.refreshDetail()]);
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      busy = false;
    }
  }
</script>

<Modal bind:open title={worker ? "Edit worker" : "Hire a worker"} icon="users" size="md">
  <p class="mb-4 text-sm text-muted">{worker ? "Where it is in the organisation. Its tools, access and versions stay as they are." : "Give an agent a role, a place in the organisation and a level of autonomy."}</p>
  <div class="flex flex-col gap-4">
    {#if !worker}
      <Field label="Agent" hint={free.length === 0 ? "Every agent is already a worker. Make one in Agents first." : "Any agent that is not a worker yet. Its tools and access do not change."} required>
        {#snippet children(f)}
          <Select id={f.id} bind:value={agent}>
            <option value="">Choose an agent</option>
            {#each free as a (a.slug)}<option value={a.slug}>{a.emoji} {a.name}</option>{/each}
          </Select>
        {/snippet}
      </Field>
    {/if}
    <div class="grid grid-cols-2 gap-3">
      <Field label="Role" required>
        {#snippet children(f)}<Input id={f.id} bind:value={role} maxlength={60} placeholder="Engineering lead" />{/snippet}
      </Field>
      <Field label="Department" required>
        {#snippet children(f)}
          <Select id={f.id} bind:value={department}>
            {#each overview?.settings.departments ?? [] as d (d.slug)}<option value={d.slug}>{d.name}</option>{/each}
          </Select>
        {/snippet}
      </Field>
    </div>
    <Field label="Reports to" hint="Every chain ends at you.">
      {#snippet children(f)}
        <Select id={f.id} bind:value={reportsTo}>
          <option value="">You</option>
          {#each bosses as b (b.worker.agent)}<option value={b.worker.agent}>{b.name} · {b.worker.role}</option>{/each}
        </Select>
      {/snippet}
    </Field>
    <Checkbox bind:checked={manager} description="It can plan the week and write the report, and gets a schedule you can turn on.">A virtual manager</Checkbox>
    <Field label="Autonomy" hint={boss ? `Up to ${top}: a worker cannot be freer than ${boss.name}.` : "How much it may do without asking. Tools that always ask still ask."}>
      {#snippet children()}<AutonomyDial bind:level max={top} label="Autonomy" />{/snippet}
    </Field>
    <div class="grid grid-cols-2 gap-3">
      <Field label="Budget per week" hint={cap !== null ? `Up to ${money(cap)}, ${boss?.name}'s.` : "Empty means no limit."} error={budgetError}>
        {#snippet children(f)}<Input id={f.id} bind:value={budget} icon="dollar" inputmode="decimal" placeholder="25" invalid={f.invalid} />{/snippet}
      </Field>
      <Field label="Time per task" hint="Hours of work, not counting waits for a person." error={timeError}>
        {#snippet children(f)}<Input id={f.id} bind:value={hours} inputmode="decimal" invalid={f.invalid} />{/snippet}
      </Field>
    </div>
  </div>
  {#snippet footer()}
    <Button disabled={busy} onclick={() => (open = false)}>Cancel</Button>
    <Button variant="primary" icon={worker ? "check" : "plus"} disabled={!valid || busy} onclick={() => void save()}>{worker ? "Save" : "Hire"}</Button>
  {/snippet}
</Modal>
