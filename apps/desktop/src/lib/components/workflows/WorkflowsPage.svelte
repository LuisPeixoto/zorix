<script lang="ts">
  import { type WfCard, type WfMissing, type WfTemplate, errorText, workflowAddTemplate, workflowCreate, workflowSetEnabled, workflowTemplates } from "../../api.ts";
  import { store } from "../../stores/runs.svelte.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { workflows } from "../../stores/workflows.svelte.ts";
  import { Badge, Button, Callout, Drawer, EmptyState, Field, Icon, Input, Modal, PageHeader, Stat, Switch, TriggerChip } from "../../ui/index.ts";
  import { barHeights, barTone, chipKind, ms, savedText, slugify } from "../../workflow.ts";

  const overview = $derived(workflows.overview);
  const cards = $derived(overview?.cards ?? []);
  const figures = $derived(overview?.figures);

  let filter = $state("");
  let templates = $state<WfTemplate[]>([]);
  let drawer = $state(false);
  let creating = $state(false);
  let name = $state("");
  let busy = $state(false);

  // The list is alive: runs start, wait and end while it is open.
  $effect(() => {
    void workflows.refresh();
    const timer = setInterval(() => void workflows.refresh(), 3000);
    return () => clearInterval(timer);
  });

  const shown = $derived(
    cards.filter((c) => {
      const q = filter.trim().toLowerCase();
      return !q || [c.workflow.draft.name, c.workflow.slug, c.workflow.draft.group ?? "", ...c.triggers.map((t) => t.label)].some((t) => t.toLowerCase().includes(q));
    }),
  );

  const slug = $derived(slugify(name));
  const taken = $derived(cards.some((c) => c.workflow.slug === slug));
  const nameError = $derived(name.trim() && !slug ? "Use some letters or digits." : taken ? "A workflow with that name already exists." : null);

  const BAR: Record<string, string> = { ok: "bg-ok/70", warn: "bg-warn", err: "bg-err", accent: "bg-accent", muted: "bg-faint" };

  async function openTemplates() {
    drawer = true;
    if (templates.length === 0) templates = await workflowTemplates().catch(() => []);
  }

  async function addTemplate(template: WfTemplate) {
    busy = true;
    try {
      const added = await workflowAddTemplate(template.id);
      drawer = false;
      toast.ok(`${added.workflow.draft.name} added as a draft that is off.`);
      await workflows.refresh();
      note(added.missing);
      store.showWorkflows("editor", added.workflow.id);
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      busy = false;
    }
  }

  function note(missing: WfMissing[]) {
    if (missing.length) toast.info(`It needs ${missing.map((m) => (m.kind === "connection" ? `a ${m.name} connection` : `the ${m.kind} ${m.name}`)).join(" and ")}. The editor lists it.`);
  }

  async function create() {
    if (!slug || nameError) return;
    busy = true;
    try {
      const made = await workflowCreate(slug, name.trim());
      creating = false;
      name = "";
      await workflows.refresh();
      store.showWorkflows("editor", made.id);
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      busy = false;
    }
  }

  async function toggle(card: WfCard, on: boolean) {
    try {
      await workflowSetEnabled(card.workflow.id, on);
      toast.ok(on ? `${card.workflow.draft.name} is on.` : `${card.workflow.draft.name} is paused.`);
    } catch (e) {
      toast.err(errorText(e));
    }
    await workflows.refresh();
  }

  const open = (card: WfCard) => store.showWorkflows("editor", card.workflow.id);
</script>

<div class="h-full overflow-y-auto">
  <div class="mx-auto max-w-[1100px] p-6">
    <PageHeader title="Workflows" subtitle="Triggers, agents, conditions and approvals joined into steps that run on their own and stop when a person must decide." glow>
      {#snippet actions()}
        <Button icon="store" onclick={openTemplates}>Templates</Button>
        <Button variant="primary" icon="plus" onclick={() => (creating = true)}>New workflow</Button>
      {/snippet}
    </PageHeader>

    {#if workflows.error && !overview}
      <Callout tone="err" title="Workflows could not be read">{workflows.error}</Callout>
    {:else if overview && !overview.host.running}
      <Callout tone="warn" title="Schedules and webhooks are off" class="mb-5">The part of ZORIX that starts workflows by itself did not start. Workflows still run when you start them by hand.</Callout>
    {/if}

    {#if overview && cards.length === 0}
      <EmptyState icon="workflow" title="No workflows yet" text="A workflow starts on a schedule, a webhook or an event, runs agents and actions in order, and stops for a person where you say so. Start from a template.">
        {#snippet actions()}
          <Button variant="primary" icon="store" onclick={openTemplates}>Browse templates</Button>
          <Button icon="plus" onclick={() => (creating = true)}>New workflow</Button>
        {/snippet}
      </EmptyState>
    {:else if figures}
      <div class="mb-5 grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        <Stat icon="workflow" label="Active workflows" value={figures.active} detail={figures.paused ? `${figures.paused} paused` : "none paused"} />
        <Stat
          icon="play"
          label="Runs this week"
          value={figures.runs_week}
          detail={figures.finished_percent === null ? "none has ended yet" : `${figures.finished_percent}% finished`}
        />
        <Stat
          icon="shield"
          label="Waiting for people"
          value={figures.waiting}
          detail={figures.oldest_wait_ms === null ? "nothing is waiting" : `oldest ${ms(Math.max(figures.oldest_wait_ms, 60_000))}`}
        />
        <Stat
          icon="clock"
          label="Time saved"
          value={figures.saved_minutes === null ? "—" : savedText(figures.saved_minutes)}
          detail={figures.saved_minutes === null ? "set minutes saved per run in a workflow" : "estimated, this week"}
        />
      </div>

      <section aria-label="All workflows" class="overflow-hidden rounded-2xl border border-line bg-panel shadow-card">
        <header class="flex items-center gap-3 border-b border-line-soft bg-panel-2 px-4 py-3">
          <Icon name="list" size={15} class="text-accent" />
          <h2 class="flex-1 text-base font-semibold">All workflows</h2>
          <div class="w-60"><Input bind:value={filter} icon="search" placeholder="Filter" size="sm" aria-label="Filter workflows" /></div>
        </header>
        <div class="overflow-x-auto">
          <table class="w-full min-w-[860px] text-left text-base">
            <thead>
              <tr class="border-b border-line-soft text-2xs font-semibold tracking-wider text-faint uppercase">
                <th scope="col" class="px-4 py-2.5 font-semibold">Workflow</th>
                <th scope="col" class="px-3 py-2.5 font-semibold">Starts on</th>
                <th scope="col" class="px-3 py-2.5 font-semibold">Last 14 runs</th>
                <th scope="col" class="px-3 py-2.5 font-semibold">Version</th>
                <th scope="col" class="px-3 py-2.5 font-semibold">State</th>
                <th scope="col" class="px-3 py-2.5 font-semibold">On</th>
                <th scope="col" class="w-10 px-3 py-2.5"><span class="sr-only">Open</span></th>
              </tr>
            </thead>
            <tbody class="divide-y divide-line-soft">
              {#each shown as card (card.workflow.id)}
                {@const heights = barHeights(card.bars)}
                <tr class="transition duration-150 hover:bg-hover">
                  <td class="px-4 py-3">
                    <button type="button" class="block min-w-0 text-left outline-none focus-visible:underline" onclick={() => open(card)}>
                      <span class="block truncate font-medium">{card.workflow.draft.name}</span>
                      <span class="block truncate text-xs text-muted">{card.workflow.draft.group || "No group"}{card.unpublished_changes && card.workflow.live_version ? " · unpublished changes" : ""}</span>
                    </button>
                    {#if card.missed > 0}<p class="mt-0.5 text-xs text-warn">Missed {card.missed} {card.missed === 1 ? "run" : "runs"} while ZORIX was closed</p>{/if}
                  </td>
                  <td class="px-3 py-3">
                    <div class="flex flex-wrap gap-1.5">
                      {#each card.triggers as trigger (trigger.step)}
                        <TriggerChip kind={chipKind(trigger.kind === "after_workflow" ? "after_workflow" : (trigger.kind as "manual"))} label={trigger.label} />
                      {/each}
                    </div>
                  </td>
                  <td class="px-3 py-3">
                    {#if card.bars.length === 0}
                      <span class="text-xs text-faint">No runs yet</span>
                    {:else}
                      <span class="flex h-5.5 items-end gap-0.5" role="img" aria-label="Last {card.bars.length} runs: {card.bars.filter((b) => b.state === 'completed').length} finished, {card.bars.filter((b) => b.state === 'failed').length} failed, {card.bars.filter((b) => b.state === 'waiting' || b.waited).length} waited for a person">
                        {#each card.bars as bar, i (bar.run)}
                          <i class="block w-1.25 rounded-xs {BAR[barTone(bar)]}" style:height="{heights[i]}%" title="{bar.state}{bar.waited ? ', waited for a person' : ''} · {ms(bar.ms)}"></i>
                        {/each}
                      </span>
                    {/if}
                  </td>
                  <td class="px-3 py-3"><Badge tone="muted" variant="outline">{card.workflow.live_version ? `v${card.workflow.live_version}` : "draft"}</Badge></td>
                  <td class="px-3 py-3">
                    {#if card.health.kind === "waiting"}<Badge tone="warn" dot>{card.health.count} waiting</Badge>
                    {:else if card.health.kind === "failed"}<Badge tone="err" dot>{card.health.count} failed</Badge>
                    {:else if card.health.kind === "paused"}<Badge tone="muted">Paused</Badge>
                    {:else}<Badge tone="ok" dot>Healthy</Badge>{/if}
                  </td>
                  <td class="px-3 py-3">
                    <Switch
                      checked={card.workflow.enabled}
                      disabled={!card.workflow.live_version}
                      aria-label="{card.workflow.draft.name} {card.workflow.enabled ? 'is on' : 'is off'}"
                      title={card.workflow.live_version ? undefined : "Publish a version first"}
                      onchange={(e) => void toggle(card, e.currentTarget.checked)}
                    />
                  </td>
                  <td class="px-3 py-3 text-right">
                    <button type="button" class="grid size-7 place-items-center rounded-lg text-muted outline-none hover:bg-hover hover:text-fg focus-visible:ring-2 focus-visible:ring-accent/40" aria-label="Open {card.workflow.draft.name}" onclick={() => open(card)}>
                      <Icon name="chevron-right" size={15} />
                    </button>
                  </td>
                </tr>
              {:else}
                <tr><td colspan="7" class="px-4 py-8 text-center text-base text-muted">No workflow matches “{filter}”.</td></tr>
              {/each}
            </tbody>
          </table>
        </div>
        <footer class="border-t border-line-soft bg-panel-2 px-4 py-2.5 text-xs text-muted">
          Schedules and webhooks run while ZORIX is open, or under <span class="mono">zorix serve</span>.
          {#if cards.length > 0 && figures.waiting > 0}<span class="text-warn"> {figures.waiting} {figures.waiting === 1 ? "run is" : "runs are"} waiting for a person, see the Inbox.</span>{/if}
        </footer>
      </section>
    {/if}
  </div>
</div>

<Drawer bind:open={drawer} title="Templates" subtitle="Each one is added as a draft that is off. Read it, change it, publish it." icon="store" width="lg">
  <ul class="space-y-3 p-1">
    {#each templates as template (template.id)}
      <li class="rounded-2xl border border-line bg-panel p-4 shadow-card">
        <div class="flex items-start gap-3">
          <div class="min-w-0 flex-1">
            <h3 class="text-md font-semibold">{template.name}</h3>
            <p class="mt-1 text-base text-muted">{template.description}</p>
            <div class="mt-2.5 flex flex-wrap items-center gap-2">
              {#each template.triggers as label (label)}<TriggerChip kind={label.includes("·") ? "webhook" : label.startsWith("After") ? "workflow" : "schedule"} {label} />{/each}
              <Badge tone="muted">{template.steps} steps</Badge>
              {#if template.group}<Badge tone="muted">{template.group}</Badge>{/if}
            </div>
          </div>
          <Button variant="primary" size="sm" disabled={busy || cards.some((c) => c.workflow.slug === template.id)} onclick={() => void addTemplate(template)}>
            {cards.some((c) => c.workflow.slug === template.id) ? "Added" : "Add"}
          </Button>
        </div>
      </li>
    {/each}
  </ul>
</Drawer>

<Modal bind:open={creating} title="New workflow" icon="workflow" size="sm">
  <form
    onsubmit={(e) => {
      e.preventDefault();
      void create();
    }}
  >
    <Field label="Name" hint={slug ? `Short name: ${slug}` : "What it does, in a few words."} error={nameError}>
      {#snippet children(f)}<Input id={f.id} bind:value={name} placeholder="Incident triage" maxlength={80} invalid={f.invalid} autofocus />{/snippet}
    </Field>
    <p class="mt-3 text-sm text-muted">It starts empty, with a manual start. You add the steps in the editor.</p>
    <div class="mt-4 flex justify-end gap-2">
      <Button type="button" onclick={() => (creating = false)}>Cancel</Button>
      <Button type="submit" variant="primary" disabled={busy || !slug || Boolean(nameError)}>Create</Button>
    </div>
  </form>
</Modal>
