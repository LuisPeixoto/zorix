<script lang="ts">
  import {
    type HostInfo,
    type WfDetail,
    type WorkflowSpec,
    agentsList,
    errorText,
    teamsList,
    workflowDetail,
    workflowExport,
    workflowHost,
    workflowPublish,
    workflowRemove,
    workflowSave,
    workflowSetEnabled,
    workflowStart,
    workflowsOverview,
  } from "../../api.ts";
  import { store } from "../../stores/runs.svelte.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { workflows } from "../../stores/workflows.svelte.ts";
  import { Badge, Button, Callout, Checkbox, Field, Icon, Input, Modal, Select, Textarea, ToggleRow, TriggerChip, WorkflowCanvas } from "../../ui/index.ts";
  import {
    type History,
    KINDS,
    type Known,
    NODE_TEXT,
    NODE_TINT,
    PALETTE_GROUPS,
    addStep,
    chipKind,
    emptyHistory,
    kindInfo,
    redo,
    remember,
    triggerLabel,
    undo,
  } from "../../workflow.ts";
  import RunsDrawer from "./RunsDrawer.svelte";
  import StepInspector from "./StepInspector.svelte";
  import VersionsDrawer from "./VersionsDrawer.svelte";

  let { id }: { id: string } = $props();

  let detail = $state<WfDetail | null>(null);
  let spec = $state<WorkflowSpec | null>(null);
  let host = $state<HostInfo | null>(null);
  let known = $state<Known>({ agents: [], teams: [], workflows: [] });
  let selected = $state<string | null>(null);
  let selectedEdge = $state<{ from: string; to: string } | null>(null);
  let history = $state<History<WorkflowSpec>>(emptyHistory());
  let saving = $state<"saved" | "saving" | "error">("saved");
  let saveError = $state("");
  let loadError = $state("");
  let versions = $state(false);
  let runs = $state(false);
  let publishing = $state(false);
  let testing = $state(false);
  let removing = $state(false);
  let note = $state("");
  let confirmed = $state(false);
  let payload = $state("{}");
  let trigger = $state("");
  let busy = $state(false);

  let timer: ReturnType<typeof setTimeout> | undefined;
  let revision = 0;
  let lastKey = "";
  let lastAt = 0;
  let dirty = false;

  async function load() {
    try {
      const [fresh, agents, teams, overview, hostInfo] = await Promise.all([workflowDetail(id), agentsList(), teamsList(), workflowsOverview(), workflowHost()]);
      detail = fresh;
      spec = structuredClone($state.snapshot(fresh.workflow.draft));
      history = emptyHistory();
      known = { agents: agents.map((a) => a.slug), teams: teams.map((t) => t.team.slug), workflows: overview.cards.map((c) => c.workflow.slug).filter((s) => s !== fresh.workflow.slug) };
      host = hostInfo;
      selected = null;
      selectedEdge = null;
      loadError = "";
      if (workflows.panel === "versions") versions = true;
      if (workflows.panel === "runs") runs = true;
    } catch (e) {
      loadError = errorText(e);
    }
  }

  $effect(() => {
    void id;
    void load();
  });

  // Edits are saved as the draft a moment after the last one. A draft that is not valid yet is kept here and says why it was not saved.
  async function flush() {
    clearTimeout(timer);
    if (!spec || !dirty) return;
    const mine = ++revision;
    const snapshot = $state.snapshot(spec) as WorkflowSpec;
    saving = "saving";
    try {
      await workflowSave(id, snapshot);
      if (mine === revision) dirty = false;
      saveError = "";
      saving = "saved";
      detail = await workflowDetail(id);
    } catch (e) {
      saving = "error";
      saveError = errorText(e);
    }
  }

  function commit(next: WorkflowSpec, key?: string) {
    if (!spec) return;
    const now = Date.now();
    if (!(key && key === lastKey && now - lastAt < 1000)) history = remember(history, $state.snapshot(spec) as WorkflowSpec);
    lastKey = key ?? "";
    lastAt = now;
    spec = next;
    dirty = true;
    saving = "saving";
    clearTimeout(timer);
    timer = setTimeout(() => void flush(), 700);
  }

  function step(kind: "undo" | "redo") {
    if (!spec) return;
    const result = (kind === "undo" ? undo : redo)(history, $state.snapshot(spec) as WorkflowSpec);
    if (!result) return;
    history = result.history;
    spec = result.value;
    dirty = true;
    clearTimeout(timer);
    timer = setTimeout(() => void flush(), 700);
  }

  // Leaving the editor must not lose the last edit.
  $effect(() => () => void flush());

  function onkeydown(event: KeyboardEvent) {
    if (!(event.metaKey || event.ctrlKey) || event.key.toLowerCase() !== "z") return;
    const target = event.target as HTMLElement;
    if (target.matches("input, textarea, select")) return;
    event.preventDefault();
    step(event.shiftKey ? "redo" : "undo");
  }

  function addFromPalette(kind: (typeof KINDS)[number]["kind"]) {
    if (!spec) return;
    const added = addStep(spec, kind, known, selected ?? undefined);
    if ("error" in added) return void toast.err(added.error);
    commit(added.spec);
    selected = added.id;
    selectedEdge = null;
  }

  function setField(patch: Partial<WorkflowSpec>, key: string) {
    if (spec) commit({ ...spec, ...patch }, key);
  }

  const live = $derived(detail?.workflow.live_version ?? null);
  const nextVersion = $derived((live ?? 0) + 1);
  const changed = $derived(detail?.unpublished_changes ?? true);
  const triggers = $derived((spec?.steps ?? []).filter((s) => kindInfo(s.kind.type).cls === "trigger"));
  const needsConfirm = $derived(detail?.to_confirm ?? []);
  const titleOf = (stepId: string) => spec?.steps.find((s) => s.id === stepId)?.title ?? stepId;

  async function openPublish() {
    await flush();
    if (saving === "error") return void toast.err(`Fix this first: ${saveError}`);
    note = "";
    confirmed = false;
    publishing = true;
  }

  async function publish() {
    busy = true;
    try {
      const made = await workflowPublish(id, note, confirmed);
      publishing = false;
      toast.ok(made.written ? `Published v${made.version}.${detail?.workflow.enabled ? " The next trigger uses it." : " It is off: turn it on to let its triggers start it."}` : `Nothing changed since v${made.version}.`);
      await load();
      void workflows.refresh();
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      busy = false;
    }
  }

  async function openTest() {
    await flush();
    if (saving === "error") return void toast.err(`Fix this first: ${saveError}`);
    payload = "{}";
    trigger = triggers.find((t) => t.kind.type === "manual")?.id ?? triggers[0]?.id ?? "";
    testing = true;
  }

  async function runTest() {
    let parsed: unknown;
    try {
      parsed = JSON.parse(payload || "{}");
    } catch {
      return void toast.err("That is not JSON. Write {} for nothing, or something like {\"title\": \"Checkout fails\"}.");
    }
    busy = true;
    try {
      const run = await workflowStart(id, true, parsed, trigger || undefined);
      testing = false;
      store.showWorkflows("run", id, run.id);
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      busy = false;
    }
  }

  async function toggleOn(on: boolean) {
    try {
      await workflowSetEnabled(id, on);
      toast.ok(on ? "It is on." : "It is paused.");
      detail = await workflowDetail(id);
      void workflows.refresh();
    } catch (e) {
      toast.err(errorText(e));
      detail = await workflowDetail(id);
    }
  }

  async function remove() {
    busy = true;
    try {
      await workflowRemove(id);
      removing = false;
      toast.ok("Workflow removed. Its runs are kept.");
      store.showWorkflows();
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      busy = false;
    }
  }

  async function copyFile() {
    try {
      await navigator.clipboard.writeText(await workflowExport(id));
      toast.ok("The workflow file is on your clipboard.");
    } catch (e) {
      toast.err(errorText(e));
    }
  }

  const dollars = (micros?: number | null) => (micros ? String(micros / 1_000_000) : "");
  const toMicros = (text: string) => (Number(text) > 0 ? Math.round(Number(text) * 1_000_000) : null);
</script>

<svelte:window {onkeydown} />

{#if loadError}
  <div class="p-6"><Callout tone="err" title="This workflow could not be opened">{loadError}</Callout></div>
{:else if detail && spec}
  <div class="grid h-full grid-rows-[auto_minmax(0,1fr)]">
    <header class="flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-line bg-panel px-5 py-3">
      <div class="min-w-0">
        <nav aria-label="Breadcrumb" class="mb-0.5 flex items-center gap-1.5 text-xs text-muted">
          <button type="button" class="inline-flex items-center gap-1.5 rounded outline-none hover:text-fg focus-visible:underline" onclick={() => store.showWorkflows()}><Icon name="workflow" size={13} />Workflows</button>
          <Icon name="chevron-right" size={12} />
          <span class="truncate">{spec.group || "No group"}</span>
        </nav>
        <div class="flex flex-wrap items-center gap-2">
          <h1 class="truncate text-xl font-semibold">{spec.name}</h1>
          <Badge tone="muted">{changed || live === null ? `Draft of v${nextVersion}` : "Draft"}</Badge>
          {#if live !== null}<Badge tone={detail.workflow.enabled ? "ok" : "muted"} dot>v{live} {detail.workflow.enabled ? "live" : "published, off"}</Badge>{:else}<Badge tone="muted">Not published</Badge>{/if}
          <span class="text-xs {saving === 'error' ? 'text-err' : 'text-faint'}" aria-live="polite" title={saveError}>
            {saving === "saving" ? "Saving…" : saving === "error" ? "Not saved" : "Saved"}
          </span>
        </div>
      </div>
      <div class="flex flex-wrap items-center gap-2">
        {#each triggers as t (t.id)}
          <TriggerChip kind={chipKind(t.kind.type)} label={triggerLabel(t.kind) ?? t.title} />
        {/each}
      </div>
      <div class="ml-auto flex items-center gap-2">
        <Button variant="ghost" icon="history" onclick={() => (versions = true)}>Versions</Button>
        <Button variant="ghost" icon="list" onclick={() => (runs = true)}>Runs</Button>
        <Button icon="play" onclick={() => void openTest()}>Test run</Button>
        <Button variant="primary" icon="arrow-up" disabled={!changed && live !== null} title={!changed && live !== null ? `No changes since v${live}` : undefined} onclick={() => void openPublish()}>Publish v{nextVersion}</Button>
      </div>
    </header>

    {#if saving === "error"}
      <div class="border-b border-err/30 bg-err/8 px-5 py-2 text-sm text-err" role="alert">Not saved yet: {saveError}. Your edits are kept here until it can be.</div>
    {/if}

    <div class="grid min-h-0 grid-cols-[200px_minmax(0,1fr)_320px]">
      <aside class="min-h-0 overflow-y-auto border-r border-line bg-panel-2 p-3.5" aria-label="Steps you can add">
        {#each PALETTE_GROUPS as group (group)}
          <p class="mt-1.5 mb-1.5 text-2xs font-semibold tracking-wider text-faint uppercase first:mt-0">{group}</p>
          <ul class="mb-3 space-y-1">
            {#each KINDS.filter((k) => k.group === group) as k (k.kind)}
              <li>
                <button
                  type="button"
                  draggable="true"
                  class="flex w-full cursor-grab items-center gap-2.5 rounded-lg border border-line bg-panel px-2 py-1.5 text-left text-sm font-medium shadow-sm transition duration-150 outline-none hover:border-line-strong focus-visible:ring-2 focus-visible:ring-accent/40 disabled:opacity-50"
                  disabled={k.kind === "after_workflow" && known.workflows.length === 0}
                  title={k.kind === "after_workflow" && known.workflows.length === 0 ? "There is no other workflow to follow yet" : "Add to the canvas, or drag it there"}
                  ondragstart={(e) => e.dataTransfer?.setData("application/x-zorix-step", k.kind)}
                  onclick={() => addFromPalette(k.kind)}
                >
                  <span class="grid size-5.5 shrink-0 place-items-center rounded-md {NODE_TINT[k.cls]} {NODE_TEXT[k.cls]}"><Icon name={k.icon} size={13} /></span>
                  {k.label}
                </button>
              </li>
            {/each}
          </ul>
        {/each}
      </aside>

      <WorkflowCanvas
        {spec}
        bind:selected
        bind:selectedEdge
        {known}
        canUndo={history.past.length > 0}
        onchange={(next) => commit(next)}
        onundo={() => step("undo")}
        onerror={(m) => toast.err(m)}
        class="rounded-none border-0"
      />

      <aside class="flex min-h-0 flex-col overflow-y-auto border-l border-line bg-panel" aria-label="Settings">
        {#if selected}
          <StepInspector {spec} stepId={selected} {known} workflowId={id} hasLive={live !== null} {host} onchange={commit} onselect={(s) => (selected = s)} onerror={(m) => toast.err(m)} />
        {:else if selectedEdge}
          <div class="space-y-3 p-4">
            <p class="text-md font-semibold">An arrow</p>
            <p class="text-sm text-muted">{titleOf(selectedEdge.from)} leads into {titleOf(selectedEdge.to)}.</p>
            <Button
              variant="danger"
              icon="trash"
              onclick={() => {
                if (selectedEdge && spec) commit({ ...spec, edges: spec.edges.filter((e) => !(e.from === selectedEdge?.from && e.to === selectedEdge?.to)) });
                selectedEdge = null;
              }}>Remove this arrow</Button
            >
          </div>
        {:else}
          <div class="space-y-4 p-4">
            <div>
              <p class="text-md font-semibold">This workflow</p>
              <p class="mt-0.5 text-xs text-muted">Select a step to change it. Edits save as the draft.</p>
            </div>
            {#if detail.missing.length > 0}
              <Callout tone="warn" title="Not set up here yet">
                <ul class="mt-1 space-y-0.5 text-sm">
                  {#each detail.missing as m (m.step + m.name)}
                    <li>{m.kind === "connection" ? `A ${m.name} connection` : `The ${m.kind} ${m.name}`} <span class="text-muted">for “{titleOf(m.step)}”</span></li>
                  {/each}
                </ul>
              </Callout>
            {/if}
            <Field label="Name">
              {#snippet children(f)}<Input id={f.id} value={spec?.name} maxlength={80} oninput={(e) => setField({ name: e.currentTarget.value }, "name")} />{/snippet}
            </Field>
            <Field label="What it is for">
              {#snippet children(f)}<Textarea id={f.id} rows={2} value={spec?.description ?? ""} maxlength={280} oninput={(e) => setField({ description: e.currentTarget.value }, "description")} />{/snippet}
            </Field>
            <Field label="Group" hint="Shown under the name in the list.">
              {#snippet children(f)}<Input id={f.id} value={spec?.group ?? ""} maxlength={40} oninput={(e) => setField({ group: e.currentTarget.value }, "group")} />{/snippet}
            </Field>
            <div class="grid grid-cols-2 gap-3">
              <Field label="Budget per run ($)" hint="Empty: no limit.">
                {#snippet children(f)}<Input id={f.id} type="number" min="0" step="0.1" value={dollars(spec?.budget_micros)} oninput={(e) => setField({ budget_micros: toMicros(e.currentTarget.value) }, "budget")} />{/snippet}
              </Field>
              <Field label="Minutes saved per run" hint="For the Time saved figure.">
                {#snippet children(f)}<Input id={f.id} type="number" min="0" value={spec?.saves_minutes ? String(spec.saves_minutes) : ""} oninput={(e) => setField({ saves_minutes: Number(e.currentTarget.value) > 0 ? Number(e.currentTarget.value) : null }, "saves")} />{/snippet}
              </Field>
            </div>
            <ToggleRow
              label="On"
              description={live === null ? "Publish a version first" : detail.workflow.enabled ? "Its triggers start it" : "Paused: nothing starts it"}
              checked={detail.workflow.enabled}
              disabled={live === null}
              onchange={(on) => void toggleOn(on)}
            />
            <div class="flex flex-wrap gap-2 border-t border-line-soft pt-4">
              <Button size="sm" icon="copy" onclick={() => void copyFile()}>Copy file</Button>
              <Button size="sm" variant="danger" icon="trash" onclick={() => (removing = true)}>Remove</Button>
            </div>
          </div>
        {/if}
      </aside>
    </div>
  </div>

  <VersionsDrawer bind:open={versions} {id} {detail} onchanged={() => void load()} />
  <RunsDrawer bind:open={runs} {id} />

  <Modal bind:open={publishing} title="Publish v{nextVersion}" icon="arrow-up">
    <p class="text-base text-muted">The next trigger uses v{nextVersion}. Runs that are already going keep the version they started with.</p>
    <div class="mt-4 space-y-4">
      <Field label="What changed (optional)">
        {#snippet children(f)}<Input id={f.id} bind:value={note} maxlength={120} placeholder="Ask the researcher about severity" />{/snippet}
      </Field>
      {#if detail.missing.length > 0}
        <Callout tone="warn" title="Not set up here yet">
          {detail.missing.map((m) => (m.kind === "connection" ? `a ${m.name} connection` : `the ${m.kind} ${m.name}`)).join(", ")}. A run that reaches the step that needs it will fail until it is.
        </Callout>
      {/if}
      {#if needsConfirm.length > 0}
        <Callout tone="warn" title="These steps send without asking first">
          <ul class="mt-1 list-disc space-y-0.5 pl-5 text-sm">
            {#each needsConfirm as stepId (stepId)}<li>{titleOf(stepId)}</li>{/each}
          </ul>
          <div class="mt-2.5"><Checkbox bind:checked={confirmed}>I confirm they may run unattended. The version records that I did.</Checkbox></div>
        </Callout>
      {/if}
    </div>
    <div class="mt-5 flex justify-end gap-2">
      <Button onclick={() => (publishing = false)}>Cancel</Button>
      <Button variant="primary" icon="arrow-up" disabled={busy || (needsConfirm.length > 0 && !confirmed)} onclick={() => void publish()}>Publish v{nextVersion}</Button>
    </div>
  </Modal>

  <Modal bind:open={testing} title="Test run" icon="play">
    <p class="text-base text-muted">This runs the draft as it is now. Every action asks first, whatever its step says, and it does not count in the list.</p>
    <div class="mt-4 space-y-4">
      {#if triggers.length > 1}
        <Field label="Start from">
          {#snippet children(f)}
            <Select id={f.id} bind:value={trigger}>{#each triggers as t (t.id)}<option value={t.id}>{triggerLabel(t.kind) ?? t.title}</option>{/each}</Select>
          {/snippet}
        </Field>
      {/if}
      <Field label="What the trigger carries (JSON)" hint="A webhook's body, for example. Steps read it as trigger.payload.">
        {#snippet children(f)}<Textarea id={f.id} mono rows={5} bind:value={payload} />{/snippet}
      </Field>
    </div>
    <div class="mt-5 flex justify-end gap-2">
      <Button onclick={() => (testing = false)}>Cancel</Button>
      <Button variant="primary" icon="play" disabled={busy} onclick={() => void runTest()}>Run test</Button>
    </div>
  </Modal>

  <Modal bind:open={removing} title="Remove this workflow?" icon="trash" size="sm">
    <p class="text-base text-muted">Its versions are deleted. Runs that already happened stay, with the steps they ran.</p>
    <div class="mt-5 flex justify-end gap-2">
      <Button onclick={() => (removing = false)}>Keep it</Button>
      <Button variant="danger" icon="trash" disabled={busy} onclick={() => void remove()}>Remove</Button>
    </div>
  </Modal>
{/if}
