<script lang="ts">
  import { t as tr } from "../../i18n/index.ts";
  import { type Day, type HostInfo, type StepKind, type WfStepHistory, type WorkflowSpec, errorText, workflowStepHistory, workflowWebhook } from "../../api.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { Button, Callout, Chip, Field, Icon, IconButton, Input, Segmented, Select, Switch, Textarea, ToggleRow } from "../../ui/index.ts";
  import {
    DAYS,
    type Known,
    NODE_TEXT,
    NODE_TINT,
    type Unit,
    WEEKDAYS,
    disconnect,
    fromUnit,
    connect,
    kindInfo,
    kindLine,
    ms,
    patchStep,
    reaches,
    relabel,
    removeStep,
    showOptions,
    toUnit,
  } from "../../workflow.ts";
  import { store } from "../../stores/runs.svelte.ts";

  let {
    spec,
    stepId,
    known,
    workflowId,
    hasLive,
    host,
    onchange,
    onselect,
    onerror,
  }: {
    spec: WorkflowSpec;
    stepId: string;
    known: Known;
    workflowId: string;
    hasLive: boolean;
    host: HostInfo | null;
    onchange: (spec: WorkflowSpec, key?: string) => void;
    onselect: (id: string | null) => void;
    onerror: (message: string) => void;
  } = $props();

  const step = $derived(spec.steps.find((s) => s.id === stepId));
  const kind = $derived(step?.kind as StepKind);
  const info = $derived(step ? kindInfo(step.kind.type) : null);

  /** Change settings of this step. `key` lets typing in one field be undone as one change. */
  function set(patch: Record<string, unknown>, key = Object.keys(patch).join(",")) {
    if (!step) return;
    onchange(patchStep(spec, step.id, { kind: { ...step.kind, ...patch } as StepKind }), `${step.id}:${key}`);
  }
  const setKind = (next: StepKind) => step && onchange(patchStep(spec, step.id, { kind: next }));
  const orNull = (text: string): string | null => (text.trim() ? text : null);
  const usd = (micros?: number | null): string => (micros ? String(micros / 1_000_000) : "");
  const micros = (text: string): number | null => {
    const n = Number(text);
    return text.trim() && Number.isFinite(n) && n > 0 ? Math.round(n * 1_000_000) : null;
  };

  // ---- What this step led to before ------------------------------------------------------------------------------------
  let history = $state<WfStepHistory | null>(null);
  $effect(() => {
    history = null;
    if (!hasLive || !step) return;
    const id = step.id;
    void workflowStepHistory(workflowId, id).then((h) => (history = h)).catch(() => (history = null));
  });
  const OUTCOME_BAR: Record<string, string> = { done: "bg-ok/70", failed: "bg-err", waiting: "bg-warn", skipped: "bg-faint", running: "bg-accent", pending: "bg-faint" };

  // ---- Webhook address -------------------------------------------------------------------------------------------------
  let address = $state<{ url: string; signing_secret: string | null; listening: boolean } | null>(null);
  let reveal = $state(false);
  $effect(() => {
    address = null;
    reveal = false;
    if (kind?.type !== "webhook") return;
    void workflowWebhook(workflowId).then((a) => (address = a)).catch(() => (address = null));
  });
  const masked = (url: string) => url.replace(/\/hooks\/([^/]+)\/.+$/, "/hooks/$1/••••••••••••");
  async function copy(text: string, done: string) {
    try {
      await navigator.clipboard.writeText(text);
      toast.ok(done);
    } catch (e) {
      toast.err(errorText(e));
    }
  }
  async function newAddress() {
    try {
      address = await workflowWebhook(workflowId, true);
      toast.ok(tr("workflows.stepInspector.newAddressMadeThe"));
    } catch (e) {
      toast.err(errorText(e));
    }
  }

  // ---- Arrows ----------------------------------------------------------------------------------------------------------
  const outgoing = $derived(spec.edges.filter((e) => e.from === stepId));
  const candidates = $derived(
    spec.steps.filter(
      (s) => s.id !== stepId && kindInfo(s.kind.type).cls !== "trigger" && !spec.edges.some((e) => e.from === stepId && e.to === s.id) && !reaches(spec, s.id, stepId),
    ),
  );
  let target = $state("");
  const titleOf = (id: string) => spec.steps.find((s) => s.id === id)?.title ?? id;
  function addArrow() {
    const joined = connect(spec, stepId, target);
    if ("error" in joined) return onerror(joined.error);
    onchange(joined.spec);
    target = "";
  }
  function remove() {
    const removed = removeStep(spec, stepId);
    if ("error" in removed) return onerror(removed.error);
    onchange(removed.spec);
    onselect(null);
  }

  // ---- Small pieces ----------------------------------------------------------------------------------------------------
  const timesText = $derived(kind?.type === "schedule" && kind.when.type === "daily" ? kind.when.times.join(", ") : "");
  function setTimes(text: string) {
    if (kind?.type !== "schedule" || kind.when.type !== "daily") return;
    const times = text.split(/[,\s]+/).map((t) => t.trim()).filter(Boolean);
    set({ when: { ...kind.when, times: times.length ? times : ["08:00"] } });
  }
  function toggleDay(day: Day) {
    if (kind?.type !== "schedule" || kind.when.type !== "daily") return;
    const days = kind.when.days.includes(day) ? kind.when.days.filter((d) => d !== day) : [...kind.when.days, day];
    set({ when: { ...kind.when, days: DAYS.filter((d) => days.includes(d)) } });
  }
  const whenMode = $derived(kind?.type === "schedule" ? kind.when.type : "daily");
  function setWhenMode(mode: string) {
    if (kind?.type !== "schedule") return;
    set({ when: mode === "every" ? { type: "every", minutes: 60 } : mode === "cron" ? { type: "cron", expr: "0 8 * * 1-5" } : { type: "daily", times: ["08:00"], days: [...WEEKDAYS] } });
  }
  const showing = $derived(kind?.type === "approval" ? new Set(kind.show ?? []) : new Set<string>());
  function toggleShow(path: string) {
    if (kind?.type !== "approval") return;
    const next = new Set(showing);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    set({ show: [...next] });
  }
  const eventMode = $derived(kind?.type === "after_workflow" ? "workflow" : "agent");
  function setEventMode(mode: string) {
    if (kind?.type !== "event" && kind?.type !== "after_workflow") return;
    if (mode === "workflow") setKind({ type: "after_workflow", workflow: known.workflows[0] ?? "", on: kind.on });
    else setKind({ type: "event", on: kind.on, agent: null });
  }
  const optionsOf = (list: string[], current: string) => (current && !list.includes(current) ? [current, ...list] : list);
</script>

{#snippet duration(label: string, value: number | null | undefined, onset: (ms: number | null) => void, empty = "No time-out", id = "")}
  {@const parts = value ? toUnit(value) : null}
  <Field label={label}>
    {#snippet children(f)}
      <div class="flex gap-2">
        <Input id={f.id} type="number" min="1" value={parts ? String(parts.amount) : ""} placeholder={empty} class="min-w-0 flex-1" oninput={(e) => onset(e.currentTarget.value ? fromUnit(Number(e.currentTarget.value), parts?.unit ?? "h") : null)} />
        <Select value={parts?.unit ?? "h"} class="w-20" aria-label="{label} unit{id}" onchange={(e) => onset(parts ? fromUnit(parts.amount, e.currentTarget.value as Unit) : fromUnit(1, e.currentTarget.value as Unit))}>
          <option value="min">min</option>
          <option value="h">h</option>
          <option value="d">d</option>
        </Select>
      </div>
    {/snippet}
  </Field>
{/snippet}

{#snippet askRow(ask: "always" | "never" | undefined)}
  <div>
    <p class="mb-1.5 text-sm font-medium">{tr("workflows.stepInspector.whenItRuns")}</p>
    <Segmented label={tr("workflows.stepInspector.askFirstOrNot")} bind:value={() => ask ?? "always", (v) => set({ ask: v })} options={[{ value: "always", label: "A person approves" }, { value: "never", label: "Runs without asking" }]} />
    {#if (ask ?? "always") === "never"}
      <p class="mt-1.5 text-xs text-warn">{tr("workflows.stepInspector.aPersonConfirmsThis")}</p>
    {:else}
      <p class="mt-1.5 text-xs text-muted">{tr("workflows.stepInspector.theRunStopsHere")}</p>
    {/if}
  </div>
{/snippet}

{#if step && info}
  <div class="flex items-center gap-2.5 border-b border-line-soft px-4 py-3.5">
    <span class="grid size-8 shrink-0 place-items-center rounded-lg {NODE_TINT[info.cls]} {NODE_TEXT[info.cls]}"><Icon name={info.icon} size={16} /></span>
    <div class="min-w-0 flex-1">
      <p class="truncate text-md font-semibold">{step.title}</p>
      <p class="truncate text-xs text-muted">{kindLine(step)}</p>
    </div>
    <IconButton icon="trash" label={tr("workflows.stepInspector.removeThisStep")} variant="danger" size="sm" onclick={remove} />
  </div>

  <div class="space-y-4 overflow-y-auto p-4">
    <Field label={tr("common.title")} hint={tr("workflows.stepInspector.sayWhatTheStep")}>
      {#snippet children(f)}<Input id={f.id} value={step.title} maxlength={80} oninput={(e) => onchange(patchStep(spec, step.id, { title: e.currentTarget.value }), `${step.id}:title`)} />{/snippet}
    </Field>

    {#if kind.type === "schedule"}
      <div>
        <p class="mb-1.5 text-sm font-medium">{tr("workflows.stepInspector.repeats")}</p>
        <Segmented label={tr("workflows.stepInspector.kindOfSchedule")} bind:value={() => whenMode, setWhenMode} options={[{ value: "daily", label: "Daily" }, { value: "every", label: "Every" }, { value: "cron", label: "Cron" }]} />
      </div>
      {#if kind.when.type === "daily"}
        <Field label={tr("workflows.stepInspector.at")} hint={tr("workflows.stepInspector.24HourTimesSeparated")}>
          {#snippet children(f)}<Input id={f.id} value={timesText} placeholder="08:00, 16:00" onchange={(e) => setTimes(e.currentTarget.value)} />{/snippet}
        </Field>
        <div>
          <p class="mb-1.5 text-sm font-medium">{tr("common.on")}</p>
          <div class="flex flex-wrap gap-1.5" role="group" aria-label={tr("workflows.stepInspector.days")}>
            {#each DAYS as day (day)}<Chip selected={kind.when.days.includes(day)} onclick={() => toggleDay(day)}>{day[0].toUpperCase() + day.slice(1)}</Chip>{/each}
          </div>
          <p class="mt-1.5 text-xs text-muted">{kind.when.days.length === 0 ? tr("workflows.stepInspector.everyDay") : ""}</p>
        </div>
      {:else if kind.when.type === "every"}
        {@const minutes = kind.when.minutes}
        <Field label={tr("workflows.stepInspector.every")} hint={tr("workflows.stepInspector.atLeast5Minutes")}>
          {#snippet children(f)}
            <div class="flex gap-2">
              <Input id={f.id} type="number" min="1" value={String(minutes % 60 === 0 && minutes >= 60 ? minutes / 60 : minutes)} class="min-w-0 flex-1" oninput={(e) => set({ when: { type: "every", minutes: Number(e.currentTarget.value) * (minutes % 60 === 0 && minutes >= 60 ? 60 : 1) } })} />
              <Select value={minutes % 60 === 0 && minutes >= 60 ? "h" : "min"} class="w-20" aria-label={tr("workflows.stepInspector.unit")} onchange={(e) => set({ when: { type: "every", minutes: e.currentTarget.value === "h" ? Math.max(1, Math.round(minutes / 60)) * 60 : Math.max(5, minutes) } })}>
                <option value="min">min</option>
                <option value="h">h</option>
              </Select>
            </div>
          {/snippet}
        </Field>
      {:else}
        <Field label={tr("workflows.stepInspector.cronExpression")} hint={tr("workflows.stepInspector.fiveFieldsMinuteHour")}>
          {#snippet children(f)}<Input id={f.id} mono value={kind.when.type === "cron" ? kind.when.expr : ""} oninput={(e) => set({ when: { type: "cron", expr: e.currentTarget.value } })} />{/snippet}
        </Field>
      {/if}
      <Field label={tr("workflows.stepInspector.timeZone")} hint={tr("workflows.stepInspector.leaveEmptyToUse")}>
        {#snippet children(f)}<Input id={f.id} value={kind.timezone ?? ""} placeholder={tr("workflows.stepInspector.europeLisbon")} oninput={(e) => set({ timezone: orNull(e.currentTarget.value) })} />{/snippet}
      </Field>
    {:else if kind.type === "webhook"}
      <Field label={tr("workflows.stepInspector.comesFrom")}>
        {#snippet children(f)}<Input id={f.id} value={kind.source} maxlength={40} oninput={(e) => set({ source: e.currentTarget.value })} />{/snippet}
      </Field>
      <Field label={tr("workflows.stepInspector.whatHappens")} hint={tr("workflows.stepInspector.aFewWordsFor")}>
        {#snippet children(f)}<Input id={f.id} value={kind.label ?? ""} maxlength={60} oninput={(e) => set({ label: orNull(e.currentTarget.value) })} />{/snippet}
      </Field>
      <Field label={tr("workflows.stepInspector.onlyWhen")} hint={tr("workflows.stepInspector.aConditionOverThe")}>
        {#snippet children(f)}<Textarea id={f.id} mono rows={2} value={kind.filter ?? ""} placeholder={tr("workflows.stepInspector.actionLabeledAndLabel")} oninput={(e) => set({ filter: orNull(e.currentTarget.value) })} />{/snippet}
      </Field>
      <ToggleRow label={tr("workflows.stepInspector.checkTheSignature")} description={tr("workflows.stepInspector.theSenderSignsEach")} checked={kind.signed ?? false} onchange={(on) => set({ signed: on })} />
      <div>
        <p class="mb-1.5 text-sm font-medium">{tr("common.address")}</p>
        {#if host && !host.webhooks}
          <Callout tone="warn" title={tr("workflows.stepInspector.webhooksAreOff")}>
            {tr("workflows.stepInspector.nothingListensYetTurn")} <button type="button" class="text-accent underline" onclick={() => store.showSettings()}>{tr("common.settings")}</button>.
          </Callout>
        {/if}
        {#if address}
          <div class="mt-2 flex items-center gap-1.5 rounded-lg border border-line bg-panel-2 py-1 pr-1 pl-2.5">
            <span class="mono selectable min-w-0 flex-1 truncate text-xs" title={reveal ? address.url : tr("workflows.stepInspector.showTheAddressTo")}>{reveal ? address.url : masked(address.url)}</span>
            <IconButton icon={reveal ? "eye" : "lock"} label={reveal ? tr("workflows.stepInspector.hideTheAddress") : tr("workflows.stepInspector.showTheAddress")} size="sm" onclick={() => (reveal = !reveal)} />
            <IconButton icon="copy" label={tr("workflows.stepInspector.copyTheAddress")} size="sm" onclick={() => void copy(address?.url ?? "", tr("workflows.stepInspector.copiedAddress"))} />
          </div>
          <p class="mt-1.5 text-xs text-muted">{tr("workflows.stepInspector.anyoneWhoHasThis")} <button type="button" class="text-accent underline" onclick={() => void newAddress()}>{tr("workflows.stepInspector.makeANewOne")}</button> {tr("workflows.stepInspector.ifItLeaks")}</p>
          {#if address.signing_secret}
            <div class="mt-2 flex items-center gap-1.5 rounded-lg border border-line bg-panel-2 py-1 pr-1 pl-2.5">
              <span class="min-w-0 flex-1 truncate text-xs text-muted">{tr("workflows.stepInspector.signingSecret")}</span>
              <IconButton icon="copy" label={tr("workflows.stepInspector.copyTheSigningSecret")} size="sm" onclick={() => void copy(address?.signing_secret ?? "", tr("workflows.stepInspector.copiedSigningSecret"))} />
            </div>
          {/if}
        {/if}
      </div>
    {:else if kind.type === "event" || kind.type === "after_workflow"}
      <div>
        <p class="mb-1.5 text-sm font-medium">{tr("workflows.stepInspector.startsWhen")}</p>
        <Segmented label={tr("workflows.stepInspector.whatEnds")} bind:value={() => eventMode, setEventMode} options={[{ value: "agent", label: "A run ends" }, { value: "workflow", label: "A workflow ends" }]} />
      </div>
      {#if kind.type === "event"}
        <Field label={tr("workflows.stepInspector.whichRun")}>
          {#snippet children(f)}
            <Select id={f.id} value={kind.agent ?? ""} onchange={(e) => set({ agent: orNull(e.currentTarget.value) })}>
              <option value="">{tr("workflows.stepInspector.anyAgentOrTeam")}</option>
              {#each optionsOf(known.agents, kind.agent ?? "") as slug (slug)}<option value={slug}>{slug}</option>{/each}
            </Select>
          {/snippet}
        </Field>
      {:else}
        <Field label={tr("workflows.stepInspector.whichWorkflow")}>
          {#snippet children(f)}
            <Select id={f.id} value={kind.workflow} onchange={(e) => set({ workflow: e.currentTarget.value })}>
              {#each optionsOf(known.workflows, kind.workflow) as slug (slug)}<option value={slug}>{slug}</option>{/each}
            </Select>
          {/snippet}
        </Field>
      {/if}
      <Field label={tr("workflows.stepInspector.ended")}>
        {#snippet children(f)}
          <Select id={f.id} value={kind.on} onchange={(e) => set({ on: e.currentTarget.value })}>
            <option value="finished">{tr("workflows.stepInspector.well")}</option>
            <option value="failed">{tr("workflows.stepInspector.withAFailure")}</option>
            <option value="any">{tr("workflows.stepInspector.eitherWay")}</option>
          </Select>
        {/snippet}
      </Field>
    {:else if kind.type === "manual"}
      <p class="text-sm text-muted">{tr("workflows.stepInspector.startsWhenYouPress")}</p>
    {:else if kind.type === "agent" || kind.type === "team"}
      {@const isAgent = kind.type === "agent"}
      <Field label={isAgent ? tr("common.agent") : tr("workflows.stepInspector.team")}>
        {#snippet children(f)}
          <Select id={f.id} value={kind.type === "agent" ? kind.agent : kind.team} onchange={(e) => set(isAgent ? { agent: e.currentTarget.value } : { team: e.currentTarget.value })}>
            {#each optionsOf(isAgent ? known.agents : known.teams, kind.type === "agent" ? kind.agent : kind.team) as slug (slug)}<option value={slug}>{slug}</option>{/each}
          </Select>
        {/snippet}
      </Field>
      {#if (isAgent ? known.agents : known.teams).length > 0 && !(isAgent ? known.agents : known.teams).includes(kind.type === "agent" ? kind.agent : kind.team)}
        <p class="-mt-2 text-xs text-warn">{tr("workflows.stepInspector.thereIsNoCalled", { value: isAgent ? "agent" : "team", team: kind.type === "agent" ? kind.agent : kind.team })}</p>
      {/if}
      <Field label={tr("common.whatToAsk")} hint={tr("workflows.stepInspector.useTriggerPayloadTitle", { value: '{{', value2: '}}' })}>
        {#snippet children(f)}<Textarea id={f.id} rows={5} value={kind.prompt} oninput={(e) => set({ prompt: e.currentTarget.value })} />{/snippet}
      </Field>
      <div class="grid grid-cols-2 gap-3">
        {#if kind.type === "agent"}
          <Field label={tr("workflows.stepInspector.version")} hint={tr("workflows.stepInspector.emptyTheNewest")}>
            {#snippet children(f)}<Input id={f.id} type="number" min="1" value={kind.version ? String(kind.version) : ""} oninput={(e) => set({ version: e.currentTarget.value ? Number(e.currentTarget.value) : null })} />{/snippet}
          </Field>
        {/if}
        <Field label={tr("workflows.stepInspector.costLimit")} hint={tr("workflows.stepInspector.emptyTheWorkflowS")}>
          {#snippet children(f)}<Input id={f.id} type="number" min="0" step="0.05" value={usd(kind.max_cost_micros)} oninput={(e) => set({ max_cost_micros: micros(e.currentTarget.value) })} />{/snippet}
        </Field>
      </div>
      <Field label={tr("common.model")} hint={tr("workflows.stepInspector.emptyTheAgentS")}>
        {#snippet children(f)}<Input id={f.id} mono value={kind.model ?? ""} placeholder="provider/model" oninput={(e) => set({ model: orNull(e.currentTarget.value) })} />{/snippet}
      </Field>
    {:else if kind.type === "condition"}
      <Field label={tr("workflows.stepInspector.isThisTrue")} hint={tr("workflows.stepInspector.yesGoesOneWay")}>
        {#snippet children(f)}<Textarea id={f.id} mono rows={3} value={kind.expr} oninput={(e) => set({ expr: e.currentTarget.value })} />{/snippet}
      </Field>
      <p class="text-xs text-muted">{tr("workflows.stepInspector.compareWith")} <span class="mono">== != &lt; &lt;= &gt; &gt;=</span> and <span class="mono">contains</span>{tr("workflows.stepInspector.joinWith")} <span class="mono">and</span>, <span class="mono">or</span>, <span class="mono">not</span>{tr("workflows.stepInspector.example")} <span class="mono">{tr("workflows.stepInspector.severity3")}</span>.</p>
    {:else if kind.type === "loop"}
      <Field label={tr("workflows.stepInspector.forEachItemOf")} hint={tr("workflows.stepInspector.aListFromAn")}>
        {#snippet children(f)}<Input id={f.id} mono value={kind.over} oninput={(e) => set({ over: e.currentTarget.value })} />{/snippet}
      </Field>
      <Field label={tr("common.agent")}>
        {#snippet children(f)}
          <Select id={f.id} value={kind.agent} onchange={(e) => set({ agent: e.currentTarget.value })}>
            {#each optionsOf(known.agents, kind.agent) as slug (slug)}<option value={slug}>{slug}</option>{/each}
          </Select>
        {/snippet}
      </Field>
      <Field label={tr("workflows.stepInspector.whatToAskFor")} hint={tr("workflows.stepInspector.itemIsTheItem", { value: '{{', value2: '}}' })}>
        {#snippet children(f)}<Textarea id={f.id} rows={4} value={kind.prompt} oninput={(e) => set({ prompt: e.currentTarget.value })} />{/snippet}
      </Field>
    {:else if kind.type === "wait"}
      <div>
        <p class="mb-1.5 text-sm font-medium">{tr("workflows.stepInspector.waits")}</p>
        <Segmented label={tr("workflows.stepInspector.waitForOrUntil")} bind:value={() => kind.until ? "until" : "for", (v) => set(v === "until" ? { for_ms: null, until: "09:00" } : { for_ms: 10 * 60_000, until: null })} options={[{ value: "for", label: "For a while" }, { value: "until", label: "Until a time" }]} />
      </div>
      {#if kind.until}
        <Field label={tr("workflows.stepInspector.until")} hint={tr("workflows.stepInspector.theNextTimeIt")}>
          {#snippet children(f)}<Input id={f.id} type="time" value={kind.until ?? "09:00"} oninput={(e) => set({ until: e.currentTarget.value })} />{/snippet}
        </Field>
      {:else}
        {@render duration(tr("workflows.stepInspector.for"), kind.for_ms, (v) => set({ for_ms: v ?? 60_000 }), "10")}
      {/if}
    {:else if kind.type === "approval"}
      <Field label={tr("workflows.stepInspector.whatIsBeingAsked")}>
        {#snippet children(f)}<Textarea id={f.id} rows={3} value={kind.message} oninput={(e) => set({ message: e.currentTarget.value })} />{/snippet}
      </Field>
      <div>
        <p class="mb-1.5 text-sm font-medium">{tr("workflows.stepInspector.showThem")}</p>
        <div class="flex flex-wrap gap-1.5" role="group" aria-label={tr("workflows.stepInspector.whatThePersonSees")}>
          {#each showOptions(spec, step.id) as option (option.path)}<Chip selected={showing.has(option.path)} onclick={() => toggleShow(option.path)}>{option.label}</Chip>{/each}
        </div>
      </div>
      <div class="grid grid-cols-[1fr_auto] items-end gap-3">
        {@render duration(tr("workflows.stepInspector.timeOutAfter"), kind.timeout_ms, (v) => set({ timeout_ms: v }))}
        <Field label={tr("workflows.stepInspector.then")}>
          {#snippet children(f)}
            <Select id={f.id} value={kind.then ?? "refuse"} class="w-28" onchange={(e) => set({ then: e.currentTarget.value })}>
              <option value="refuse">{tr("common.refuse")}</option>
              <option value="continue">{tr("common.continue")}</option>
            </Select>
          {/snippet}
        </Field>
      </div>
      <ToggleRow
        label={tr("workflows.stepInspector.remindInSlack")}
        description={kind.remind ? tr("workflows.stepInspector.afterIn", { value: ms(kind.remind.after_ms), channel: kind.remind.channel }) : tr("workflows.stepInspector.aMessageToA")}
        checked={Boolean(kind.remind)}
        onchange={(on) => set({ remind: on ? { after_ms: 30 * 60_000, connection: "slack", channel: "#on-call" } : null })}
      />
      {#if kind.remind}
        {@const remind = kind.remind}
        <div class="space-y-3 rounded-xl border border-line bg-panel-2 p-3">
          <Field label={tr("workflows.stepInspector.connection")}>
            {#snippet children(f)}<Input id={f.id} value={remind.connection} oninput={(e) => set({ remind: { ...remind, connection: e.currentTarget.value } })} />{/snippet}
          </Field>
          <Field label={tr("workflows.stepInspector.channel")}>
            {#snippet children(f)}<Input id={f.id} value={remind.channel} oninput={(e) => set({ remind: { ...remind, channel: e.currentTarget.value } })} />{/snippet}
          </Field>
          {@render duration(tr("workflows.stepInspector.remindAfter"), remind.after_ms, (v) => set({ remind: { ...remind, after_ms: v ?? 30 * 60_000 } }), "30")}
          <p class="text-xs text-warn">{tr("workflows.stepInspector.aPersonConfirmsThis2")}</p>
        </div>
      {/if}
    {:else if kind.type === "question"}
      <Field label={tr("workflows.stepInspector.question")}>
        {#snippet children(f)}<Textarea id={f.id} rows={3} value={kind.question} oninput={(e) => set({ question: e.currentTarget.value })} />{/snippet}
      </Field>
      <Field label={tr("workflows.stepInspector.defaultAnswer")} hint={tr("workflows.stepInspector.usedWhenItTimes")}>
        {#snippet children(f)}<Input id={f.id} value={kind.default ?? ""} oninput={(e) => set({ default: orNull(e.currentTarget.value) })} />{/snippet}
      </Field>
      <div class="grid grid-cols-[1fr_auto] items-end gap-3">
        {@render duration(tr("workflows.stepInspector.timeOutAfter"), kind.timeout_ms, (v) => set({ timeout_ms: v }))}
        <Field label={tr("workflows.stepInspector.then")}>
          {#snippet children(f)}
            <Select id={f.id} value={kind.then ?? "refuse"} class="w-28" onchange={(e) => set({ then: e.currentTarget.value })}>
              <option value="refuse">{tr("common.refuse")}</option>
              <option value="continue">{tr("common.continue")}</option>
            </Select>
          {/snippet}
        </Field>
      </div>
    {:else if kind.type === "send"}
      <div>
        <p class="mb-1.5 text-sm font-medium">{tr("workflows.stepInspector.where")}</p>
        <Segmented label={tr("workflows.stepInspector.service")} bind:value={() => kind.service, (v) => set({ service: v })} options={[{ value: "slack", label: "Slack" }, { value: "discord", label: "Discord" }]} />
      </div>
      <Field label={tr("workflows.stepInspector.connection")} hint={tr("workflows.stepInspector.emptyTheOnlyOne")}>
        {#snippet children(f)}<Input id={f.id} value={kind.connection ?? ""} oninput={(e) => set({ connection: orNull(e.currentTarget.value) })} />{/snippet}
      </Field>
      <Field label={tr("workflows.stepInspector.channel")}>
        {#snippet children(f)}<Input id={f.id} value={kind.channel} oninput={(e) => set({ channel: e.currentTarget.value })} />{/snippet}
      </Field>
      <Field label={tr("workflows.stepInspector.message")} hint={tr("workflows.stepInspector.useStepsIdOutput", { value: '{{', value2: '}}' })}>
        {#snippet children(f)}<Textarea id={f.id} rows={4} value={kind.text} oninput={(e) => set({ text: e.currentTarget.value })} />{/snippet}
      </Field>
      {@render askRow(kind.ask)}
    {:else if kind.type === "http"}
      <div class="grid grid-cols-[6rem_1fr] gap-3">
        <Field label={tr("workflows.stepInspector.method")}>
          {#snippet children(f)}
            <Select id={f.id} value={kind.method} onchange={(e) => set({ method: e.currentTarget.value })}>
              {#each ["GET", "POST", "PUT", "PATCH", "DELETE"] as method (method)}<option value={method}>{method}</option>{/each}
            </Select>
          {/snippet}
        </Field>
        <Field label={tr("common.address")}>
          {#snippet children(f)}<Input id={f.id} mono value={kind.url} oninput={(e) => set({ url: e.currentTarget.value })} />{/snippet}
        </Field>
      </div>
      <Field label={tr("workflows.stepInspector.body")} hint={tr("workflows.stepInspector.textOrJsonYou")}>
        {#snippet children(f)}<Textarea id={f.id} mono rows={4} value={kind.body ?? ""} oninput={(e) => set({ body: orNull(e.currentTarget.value) })} />{/snippet}
      </Field>
      {@render askRow(kind.ask)}
    {/if}

    <div>
      <p class="mb-1.5 text-sm font-medium">{tr("workflows.stepInspector.leadsTo")}</p>
      {#if outgoing.length === 0}
        <p class="text-xs text-muted">{tr("workflows.stepInspector.nothingYetDragFrom")}</p>
      {:else}
        <ul class="space-y-1.5">
          {#each outgoing as edge (edge.to)}
            <li class="flex items-center gap-2 rounded-lg border border-line bg-panel-2 py-1 pr-1 pl-2.5">
              {#if kind.type === "condition" && edge.label}
                <Segmented label={tr("workflows.stepInspector.branchTo", { value: titleOf(edge.to) })} bind:value={() => edge.label ?? "yes", (v) => onchange(relabel(spec, step.id, edge.to, v as "yes" | "no"))} options={[{ value: "yes", label: "yes" }, { value: "no", label: "no" }]} />
              {/if}
              <span class="min-w-0 flex-1 truncate text-sm">{titleOf(edge.to)}</span>
              <IconButton icon="x" label={tr("workflows.stepInspector.removeTheArrowTo", { value: titleOf(edge.to) })} size="sm" onclick={() => onchange(disconnect(spec, step.id, edge.to))} />
            </li>
          {/each}
        </ul>
      {/if}
      {#if candidates.length > 0}
        <div class="mt-2 flex gap-2">
          <Select bind:value={target} aria-label={tr("workflows.stepInspector.addAnArrowTo")} class="min-w-0 flex-1">
            <option value="">{tr("workflows.stepInspector.addAnArrowTo2")}</option>
            {#each candidates as candidate (candidate.id)}<option value={candidate.id}>{candidate.title}</option>{/each}
          </Select>
          <Button size="sm" disabled={!target} onclick={addArrow}>{tr("common.add")}</Button>
        </div>
      {/if}
    </div>

    {#if hasLive}
      <div class="border-t border-line-soft pt-4">
        <p class="text-2xs font-semibold tracking-wider text-faint uppercase">{tr("workflows.stepInspector.last5RunsOf")}</p>
        {#if history && history.outcomes.length > 0}
          <div class="mt-2 flex items-center justify-between gap-3">
            <span class="flex h-5.5 items-end gap-0.5" role="img" aria-label={tr("workflows.stepInspector.lastRunsOfThis", { length: history.outcomes.length, value: history.outcomes.join(', ') })}>
              {#each history.outcomes as outcome, i (i)}<i class="block h-full w-1.5 rounded-xs {OUTCOME_BAR[outcome] ?? 'bg-faint'}"></i>{/each}
            </span>
            <span class="text-xs text-muted">{history.median_wait_ms === null ? tr("workflows.stepInspector.noOneWaited") : tr("workflows.stepInspector.medianWait", { value: ms(Math.max(history.median_wait_ms, 60_000)) })}</span>
          </div>
        {:else}
          <p class="mt-1.5 text-xs text-muted">{tr("workflows.stepInspector.thisStepHasNot")}</p>
        {/if}
      </div>
    {/if}
  </div>
{/if}
