<script lang="ts">
  import { t as tr } from "../../i18n/index.ts";
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
    groupLabel,
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
    if (target.matches(tr("workflows.workflowEditor.inputTextareaSelect"))) return;
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
    if (saving === "error") return void toast.err(tr("workflows.workflowEditor.fixThisFirst", { saveError }));
    note = "";
    confirmed = false;
    publishing = true;
  }

  async function publish() {
    busy = true;
    try {
      const made = await workflowPublish(id, note, confirmed);
      publishing = false;
      toast.ok(made.written ? tr("workflows.workflowEditor.publishedV", { version: made.version, value: detail?.workflow.enabled ? tr("workflows.workflowEditor.theNextTriggerUses2") : tr("workflows.workflowEditor.itIsOffTurn") }) : tr("workflows.workflowEditor.nothingChangedSinceV", { version: made.version }));
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
    if (saving === "error") return void toast.err(tr("workflows.workflowEditor.fixThisFirst", { saveError }));
    payload = "{}";
    trigger = triggers.find((t) => t.kind.type === "manual")?.id ?? triggers[0]?.id ?? "";
    testing = true;
  }

  async function runTest() {
    let parsed: unknown;
    try {
      parsed = JSON.parse(payload || "{}");
    } catch {
      return void toast.err(tr("workflows.workflowEditor.thatIsNotJson"));
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
      toast.ok(on ? tr("workflows.workflowEditor.itIsOn") : tr("workflows.workflowEditor.itIsPaused"));
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
      toast.ok(tr("workflows.workflowEditor.workflowRemovedItsRuns"));
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
      toast.ok(tr("workflows.workflowEditor.theWorkflowFileIs"));
    } catch (e) {
      toast.err(errorText(e));
    }
  }

  const dollars = (micros?: number | null) => (micros ? String(micros / 1_000_000) : "");
  const toMicros = (text: string) => (Number(text) > 0 ? Math.round(Number(text) * 1_000_000) : null);
</script>

<svelte:window {onkeydown} />

{#if loadError}
  <div class="p-6"><Callout tone="err" title={tr("workflows.workflowEditor.thisWorkflowCouldNot")}>{loadError}</Callout></div>
{:else if detail && spec}
  <div class="grid h-full grid-rows-[auto_minmax(0,1fr)]">
    <header class="flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-line bg-panel px-5 py-3">
      <div class="min-w-0">
        <nav aria-label={tr("common.breadcrumb")} class="mb-0.5 flex items-center gap-1.5 text-xs text-muted">
          <button type="button" class="inline-flex items-center gap-1.5 rounded outline-none hover:text-fg focus-visible:underline" onclick={() => store.showWorkflows()}><Icon name="workflow" size={13} />{tr("common.workflows")}</button>
          <Icon name="chevron-right" size={12} />
          <span class="truncate">{spec.group || tr("workflows.workflowEditor.noGroup")}</span>
        </nav>
        <div class="flex flex-wrap items-center gap-2">
          <h1 class="truncate text-xl font-semibold">{spec.name}</h1>
          <Badge tone="muted">{changed || live === null ? tr("workflows.workflowEditor.draftOfV", { nextVersion }) : tr("workflows.workflowEditor.draft")}</Badge>
          {#if live !== null}<Badge tone={detail.workflow.enabled ? "ok" : "muted"} dot>v{live} {detail.workflow.enabled ? "live" : tr("workflows.workflowEditor.publishedOff")}</Badge>{:else}<Badge tone="muted">{tr("workflows.workflowEditor.notPublished")}</Badge>{/if}
          <span class="text-xs {saving === 'error' ? 'text-err' : 'text-faint'}" aria-live="polite" title={saveError}>
            {saving === "saving" ? tr("workflows.workflowEditor.saving") : saving === "error" ? tr("workflows.workflowEditor.notSaved") : tr("workflows.workflowEditor.saved")}
          </span>
        </div>
      </div>
      <div class="flex flex-wrap items-center gap-2">
        {#each triggers as t (t.id)}
          <TriggerChip kind={chipKind(t.kind.type)} label={triggerLabel(t.kind) ?? t.title} />
        {/each}
      </div>
      <div class="ml-auto flex items-center gap-2">
        <Button variant="ghost" icon="history" onclick={() => (versions = true)}>{tr("common.versions")}</Button>
        <Button variant="ghost" icon="list" onclick={() => (runs = true)}>{tr("common.runs")}</Button>
        <Button icon="play" onclick={() => void openTest()}>{tr("workflows.workflowEditor.testRun")}</Button>
        <Button variant="primary" icon="arrow-up" disabled={!changed && live !== null} title={!changed && live !== null ? tr("workflows.workflowEditor.noChangesSinceV", { live }) : undefined} onclick={() => void openPublish()}>{tr("workflows.workflowEditor.publishV", { nextVersion })}</Button>
      </div>
    </header>

    {#if saving === "error"}
      <div class="border-b border-err/30 bg-err/8 px-5 py-2 text-sm text-err" role="alert">{tr("workflows.workflowEditor.notSavedYetYour", { saveError })}</div>
    {/if}

    <div class="grid min-h-0 grid-cols-[200px_minmax(0,1fr)_320px]">
      <aside class="min-h-0 overflow-y-auto border-r border-line bg-panel-2 p-3.5" aria-label={tr("workflows.workflowEditor.stepsYouCanAdd")}>
        {#each PALETTE_GROUPS as group (group)}
          <p class="mt-1.5 mb-1.5 text-2xs font-semibold tracking-wider text-faint uppercase first:mt-0">{groupLabel(group)}</p>
          <ul class="mb-3 space-y-1">
            {#each KINDS.filter((k) => k.group === group) as k (k.kind)}
              <li>
                <button
                  type="button"
                  draggable="true"
                  class="flex w-full cursor-grab items-center gap-2.5 rounded-lg border border-line bg-panel px-2 py-1.5 text-left text-sm font-medium shadow-sm transition duration-150 outline-none hover:border-line-strong focus-visible:ring-2 focus-visible:ring-accent/40 disabled:opacity-50"
                  disabled={k.kind === "after_workflow" && known.workflows.length === 0}
                  title={k.kind === "after_workflow" && known.workflows.length === 0 ? tr("workflows.workflowEditor.thereIsNoOther") : tr("workflows.workflowEditor.addToTheCanvas")}
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

      <aside class="flex min-h-0 flex-col overflow-y-auto border-l border-line bg-panel" aria-label={tr("common.settings")}>
        {#if selected}
          <StepInspector {spec} stepId={selected} {known} workflowId={id} hasLive={live !== null} {host} onchange={commit} onselect={(s) => (selected = s)} onerror={(m) => toast.err(m)} />
        {:else if selectedEdge}
          <div class="space-y-3 p-4">
            <p class="text-md font-semibold">{tr("workflows.workflowEditor.anArrow")}</p>
            <p class="text-sm text-muted">{tr("workflows.workflowEditor.leadsInto", { value: titleOf(selectedEdge.from), value2: titleOf(selectedEdge.to) })}</p>
            <Button
              variant="danger"
              icon="trash"
              onclick={() => {
                if (selectedEdge && spec) commit({ ...spec, edges: spec.edges.filter((e) => !(e.from === selectedEdge?.from && e.to === selectedEdge?.to)) });
                selectedEdge = null;
              }}>{tr("workflows.workflowEditor.removeThisArrow")}</Button
            >
          </div>
        {:else}
          <div class="space-y-4 p-4">
            <div>
              <p class="text-md font-semibold">{tr("workflows.workflowEditor.thisWorkflow")}</p>
              <p class="mt-0.5 text-xs text-muted">{tr("workflows.workflowEditor.selectAStepTo")}</p>
            </div>
            {#if detail.missing.length > 0}
              <Callout tone="warn" title={tr("workflows.workflowEditor.notSetUpHere")}>
                <ul class="mt-1 space-y-0.5 text-sm">
                  {#each detail.missing as m (m.step + m.name)}
                    <li>{m.kind === "connection" ? `A ${m.name} connection` : tr("workflows.workflowEditor.the", { kind: m.kind, name: m.name })} <span class="text-muted">{tr("workflows.workflowEditor.for", { value: titleOf(m.step) })}</span></li>
                  {/each}
                </ul>
              </Callout>
            {/if}
            <Field label={tr("common.name")}>
              {#snippet children(f)}<Input id={f.id} value={spec?.name} maxlength={80} oninput={(e) => setField({ name: e.currentTarget.value }, "name")} />{/snippet}
            </Field>
            <Field label={tr("common.whatItIsFor")}>
              {#snippet children(f)}<Textarea id={f.id} rows={2} value={spec?.description ?? ""} maxlength={280} oninput={(e) => setField({ description: e.currentTarget.value }, "description")} />{/snippet}
            </Field>
            <Field label={tr("workflows.workflowEditor.group")} hint={tr("workflows.workflowEditor.shownUnderTheName")}>
              {#snippet children(f)}<Input id={f.id} value={spec?.group ?? ""} maxlength={40} oninput={(e) => setField({ group: e.currentTarget.value }, "group")} />{/snippet}
            </Field>
            <div class="grid grid-cols-2 gap-3">
              <Field label={tr("workflows.workflowEditor.budgetPerRun")} hint={tr("workflows.workflowEditor.emptyNoLimit")}>
                {#snippet children(f)}<Input id={f.id} type="number" min="0" step="0.1" value={dollars(spec?.budget_micros)} oninput={(e) => setField({ budget_micros: toMicros(e.currentTarget.value) }, "budget")} />{/snippet}
              </Field>
              <Field label={tr("workflows.workflowEditor.minutesSavedPerRun")} hint={tr("workflows.workflowEditor.forTheTimeSaved")}>
                {#snippet children(f)}<Input id={f.id} type="number" min="0" value={spec?.saves_minutes ? String(spec.saves_minutes) : ""} oninput={(e) => setField({ saves_minutes: Number(e.currentTarget.value) > 0 ? Number(e.currentTarget.value) : null }, "saves")} />{/snippet}
              </Field>
            </div>
            <ToggleRow
              label={tr("common.on")}
              description={live === null ? tr("workflows.workflowEditor.publishAVersionFirst") : detail.workflow.enabled ? tr("workflows.workflowEditor.itsTriggersStartIt") : tr("workflows.workflowEditor.pausedNothingStartsIt")}
              checked={detail.workflow.enabled}
              disabled={live === null}
              onchange={(on) => void toggleOn(on)}
            />
            <div class="flex flex-wrap gap-2 border-t border-line-soft pt-4">
              <Button size="sm" icon="copy" onclick={() => void copyFile()}>{tr("workflows.workflowEditor.copyFile")}</Button>
              <Button size="sm" variant="danger" icon="trash" onclick={() => (removing = true)}>{tr("common.remove")}</Button>
            </div>
          </div>
        {/if}
      </aside>
    </div>
  </div>

  <VersionsDrawer bind:open={versions} {id} {detail} onchanged={() => void load()} />
  <RunsDrawer bind:open={runs} {id} />

  <Modal bind:open={publishing} title={tr("workflows.workflowEditor.publishV", { nextVersion })} icon="arrow-up">
    <p class="text-base text-muted">{tr("workflows.workflowEditor.theNextTriggerUses", { nextVersion })}</p>
    <div class="mt-4 space-y-4">
      <Field label={tr("workflows.workflowEditor.whatChangedOptional")}>
        {#snippet children(f)}<Input id={f.id} bind:value={note} maxlength={120} placeholder={tr("workflows.workflowEditor.askTheResearcherAbout")} />{/snippet}
      </Field>
      {#if detail.missing.length > 0}
        <Callout tone="warn" title={tr("workflows.workflowEditor.notSetUpHere")}>
          {tr("workflows.workflowEditor.aRunThatReaches", { value: detail.missing.map((m) => (m.kind === "connection" ? `a ${m.name} connection` : `the ${m.kind} ${m.name}`)).join(", ") })}
        </Callout>
      {/if}
      {#if needsConfirm.length > 0}
        <Callout tone="warn" title={tr("workflows.workflowEditor.theseStepsSendWithout")}>
          <ul class="mt-1 list-disc space-y-0.5 pl-5 text-sm">
            {#each needsConfirm as stepId (stepId)}<li>{titleOf(stepId)}</li>{/each}
          </ul>
          <div class="mt-2.5"><Checkbox bind:checked={confirmed}>{tr("workflows.workflowEditor.iConfirmTheyMay")}</Checkbox></div>
        </Callout>
      {/if}
    </div>
    <div class="mt-5 flex justify-end gap-2">
      <Button onclick={() => (publishing = false)}>{tr("common.cancel")}</Button>
      <Button variant="primary" icon="arrow-up" disabled={busy || (needsConfirm.length > 0 && !confirmed)} onclick={() => void publish()}>{tr("workflows.workflowEditor.publishV", { nextVersion })}</Button>
    </div>
  </Modal>

  <Modal bind:open={testing} title={tr("workflows.workflowEditor.testRun")} icon="play">
    <p class="text-base text-muted">{tr("workflows.workflowEditor.thisRunsTheDraft")}</p>
    <div class="mt-4 space-y-4">
      {#if triggers.length > 1}
        <Field label={tr("workflows.workflowEditor.startFrom")}>
          {#snippet children(f)}
            <Select id={f.id} bind:value={trigger}>{#each triggers as t (t.id)}<option value={t.id}>{triggerLabel(t.kind) ?? t.title}</option>{/each}</Select>
          {/snippet}
        </Field>
      {/if}
      <Field label={tr("workflows.workflowEditor.whatTheTriggerCarries")} hint={tr("workflows.workflowEditor.aWebhookSBody")}>
        {#snippet children(f)}<Textarea id={f.id} mono rows={5} bind:value={payload} />{/snippet}
      </Field>
    </div>
    <div class="mt-5 flex justify-end gap-2">
      <Button onclick={() => (testing = false)}>{tr("common.cancel")}</Button>
      <Button variant="primary" icon="play" disabled={busy} onclick={() => void runTest()}>{tr("workflows.workflowEditor.runTest")}</Button>
    </div>
  </Modal>

  <Modal bind:open={removing} title={tr("workflows.workflowEditor.removeThisWorkflow")} icon="trash" size="sm">
    <p class="text-base text-muted">{tr("workflows.workflowEditor.itsVersionsAreDeleted")}</p>
    <div class="mt-5 flex justify-end gap-2">
      <Button onclick={() => (removing = false)}>{tr("common.keepIt")}</Button>
      <Button variant="danger" icon="trash" disabled={busy} onclick={() => void remove()}>{tr("common.remove")}</Button>
    </div>
  </Modal>
{/if}
