<script lang="ts">
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
  async function copy(text: string, what: string) {
    try {
      await navigator.clipboard.writeText(text);
      toast.ok(`${what} copied.`);
    } catch (e) {
      toast.err(errorText(e));
    }
  }
  async function newAddress() {
    try {
      address = await workflowWebhook(workflowId, true);
      toast.ok("New address made. The old one no longer works.");
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
    <p class="mb-1.5 text-sm font-medium">When it runs</p>
    <Segmented label="Ask first or not" bind:value={() => ask ?? "always", (v) => set({ ask: v })} options={[{ value: "always", label: "A person approves" }, { value: "never", label: "Runs without asking" }]} />
    {#if (ask ?? "always") === "never"}
      <p class="mt-1.5 text-xs text-warn">A person confirms this when publishing. A test run still asks first.</p>
    {:else}
      <p class="mt-1.5 text-xs text-muted">The run stops here and shows what would be sent, until a person approves it.</p>
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
    <IconButton icon="trash" label="Remove this step" variant="danger" size="sm" onclick={remove} />
  </div>

  <div class="space-y-4 overflow-y-auto p-4">
    <Field label="Title" hint="Say what the step does.">
      {#snippet children(f)}<Input id={f.id} value={step.title} maxlength={80} oninput={(e) => onchange(patchStep(spec, step.id, { title: e.currentTarget.value }), `${step.id}:title`)} />{/snippet}
    </Field>

    {#if kind.type === "schedule"}
      <div>
        <p class="mb-1.5 text-sm font-medium">Repeats</p>
        <Segmented label="Kind of schedule" bind:value={() => whenMode, setWhenMode} options={[{ value: "daily", label: "Daily" }, { value: "every", label: "Every" }, { value: "cron", label: "Cron" }]} />
      </div>
      {#if kind.when.type === "daily"}
        <Field label="At" hint="24-hour times, separated by commas.">
          {#snippet children(f)}<Input id={f.id} value={timesText} placeholder="08:00, 16:00" onchange={(e) => setTimes(e.currentTarget.value)} />{/snippet}
        </Field>
        <div>
          <p class="mb-1.5 text-sm font-medium">On</p>
          <div class="flex flex-wrap gap-1.5" role="group" aria-label="Days">
            {#each DAYS as day (day)}<Chip selected={kind.when.days.includes(day)} onclick={() => toggleDay(day)}>{day[0].toUpperCase() + day.slice(1)}</Chip>{/each}
          </div>
          <p class="mt-1.5 text-xs text-muted">{kind.when.days.length === 0 ? "Every day." : ""}</p>
        </div>
      {:else if kind.when.type === "every"}
        {@const minutes = kind.when.minutes}
        <Field label="Every" hint="At least 5 minutes.">
          {#snippet children(f)}
            <div class="flex gap-2">
              <Input id={f.id} type="number" min="1" value={String(minutes % 60 === 0 && minutes >= 60 ? minutes / 60 : minutes)} class="min-w-0 flex-1" oninput={(e) => set({ when: { type: "every", minutes: Number(e.currentTarget.value) * (minutes % 60 === 0 && minutes >= 60 ? 60 : 1) } })} />
              <Select value={minutes % 60 === 0 && minutes >= 60 ? "h" : "min"} class="w-20" aria-label="Unit" onchange={(e) => set({ when: { type: "every", minutes: e.currentTarget.value === "h" ? Math.max(1, Math.round(minutes / 60)) * 60 : Math.max(5, minutes) } })}>
                <option value="min">min</option>
                <option value="h">h</option>
              </Select>
            </div>
          {/snippet}
        </Field>
      {:else}
        <Field label="Cron expression" hint="Five fields: minute hour day-of-month month day-of-week.">
          {#snippet children(f)}<Input id={f.id} mono value={kind.when.type === "cron" ? kind.when.expr : ""} oninput={(e) => set({ when: { type: "cron", expr: e.currentTarget.value } })} />{/snippet}
        </Field>
      {/if}
      <Field label="Time zone" hint="Leave empty to use this computer's.">
        {#snippet children(f)}<Input id={f.id} value={kind.timezone ?? ""} placeholder="Europe/Lisbon" oninput={(e) => set({ timezone: orNull(e.currentTarget.value) })} />{/snippet}
      </Field>
    {:else if kind.type === "webhook"}
      <Field label="Comes from">
        {#snippet children(f)}<Input id={f.id} value={kind.source} maxlength={40} oninput={(e) => set({ source: e.currentTarget.value })} />{/snippet}
      </Field>
      <Field label="What happens" hint="A few words for the chip, such as “issue labelled”.">
        {#snippet children(f)}<Input id={f.id} value={kind.label ?? ""} maxlength={60} oninput={(e) => set({ label: orNull(e.currentTarget.value) })} />{/snippet}
      </Field>
      <Field label="Only when" hint="A condition over the request body. Requests it rejects are ignored.">
        {#snippet children(f)}<Textarea id={f.id} mono rows={2} value={kind.filter ?? ""} placeholder={'action == "labeled" and label.name == "incident"'} oninput={(e) => set({ filter: orNull(e.currentTarget.value) })} />{/snippet}
      </Field>
      <ToggleRow label="Check the signature" description="The sender signs each request (X-Hub-Signature-256). Requests without a valid signature are refused." checked={kind.signed ?? false} onchange={(on) => set({ signed: on })} />
      <div>
        <p class="mb-1.5 text-sm font-medium">Address</p>
        {#if host && !host.webhooks}
          <Callout tone="warn" title="Webhooks are off">
            Nothing listens yet. Turn them on in <button type="button" class="text-accent underline" onclick={() => store.showSettings()}>Settings</button>.
          </Callout>
        {/if}
        {#if address}
          <div class="mt-2 flex items-center gap-1.5 rounded-lg border border-line bg-panel-2 py-1 pr-1 pl-2.5">
            <span class="mono selectable min-w-0 flex-1 truncate text-xs" title={reveal ? address.url : "Show the address to read it"}>{reveal ? address.url : masked(address.url)}</span>
            <IconButton icon={reveal ? "eye" : "lock"} label={reveal ? "Hide the address" : "Show the address"} size="sm" onclick={() => (reveal = !reveal)} />
            <IconButton icon="copy" label="Copy the address" size="sm" onclick={() => void copy(address?.url ?? "", "Address")} />
          </div>
          <p class="mt-1.5 text-xs text-muted">Anyone who has this address can start the workflow. <button type="button" class="text-accent underline" onclick={() => void newAddress()}>Make a new one</button> if it leaks.</p>
          {#if address.signing_secret}
            <div class="mt-2 flex items-center gap-1.5 rounded-lg border border-line bg-panel-2 py-1 pr-1 pl-2.5">
              <span class="min-w-0 flex-1 truncate text-xs text-muted">Signing secret</span>
              <IconButton icon="copy" label="Copy the signing secret" size="sm" onclick={() => void copy(address?.signing_secret ?? "", "Signing secret")} />
            </div>
          {/if}
        {/if}
      </div>
    {:else if kind.type === "event" || kind.type === "after_workflow"}
      <div>
        <p class="mb-1.5 text-sm font-medium">Starts when</p>
        <Segmented label="What ends" bind:value={() => eventMode, setEventMode} options={[{ value: "agent", label: "A run ends" }, { value: "workflow", label: "A workflow ends" }]} />
      </div>
      {#if kind.type === "event"}
        <Field label="Which run">
          {#snippet children(f)}
            <Select id={f.id} value={kind.agent ?? ""} onchange={(e) => set({ agent: orNull(e.currentTarget.value) })}>
              <option value="">Any agent or team</option>
              {#each optionsOf(known.agents, kind.agent ?? "") as slug (slug)}<option value={slug}>{slug}</option>{/each}
            </Select>
          {/snippet}
        </Field>
      {:else}
        <Field label="Which workflow">
          {#snippet children(f)}
            <Select id={f.id} value={kind.workflow} onchange={(e) => set({ workflow: e.currentTarget.value })}>
              {#each optionsOf(known.workflows, kind.workflow) as slug (slug)}<option value={slug}>{slug}</option>{/each}
            </Select>
          {/snippet}
        </Field>
      {/if}
      <Field label="Ended">
        {#snippet children(f)}
          <Select id={f.id} value={kind.on} onchange={(e) => set({ on: e.currentTarget.value })}>
            <option value="finished">Well</option>
            <option value="failed">With a failure</option>
            <option value="any">Either way</option>
          </Select>
        {/snippet}
      </Field>
    {:else if kind.type === "manual"}
      <p class="text-sm text-muted">Starts when you press Test run, or run it by hand from the list or the command line.</p>
    {:else if kind.type === "agent" || kind.type === "team"}
      {@const isAgent = kind.type === "agent"}
      <Field label={isAgent ? "Agent" : "Team"}>
        {#snippet children(f)}
          <Select id={f.id} value={kind.type === "agent" ? kind.agent : kind.team} onchange={(e) => set(isAgent ? { agent: e.currentTarget.value } : { team: e.currentTarget.value })}>
            {#each optionsOf(isAgent ? known.agents : known.teams, kind.type === "agent" ? kind.agent : kind.team) as slug (slug)}<option value={slug}>{slug}</option>{/each}
          </Select>
        {/snippet}
      </Field>
      {#if (isAgent ? known.agents : known.teams).length > 0 && !(isAgent ? known.agents : known.teams).includes(kind.type === "agent" ? kind.agent : kind.team)}
        <p class="-mt-2 text-xs text-warn">There is no {isAgent ? "agent" : "team"} called {kind.type === "agent" ? kind.agent : kind.team} here yet.</p>
      {/if}
      <Field label="What to ask" hint="Use {'{{'}trigger.payload.title{'}}'} or {'{{'}steps.<id>.output.text{'}}'} to pass things in.">
        {#snippet children(f)}<Textarea id={f.id} rows={5} value={kind.prompt} oninput={(e) => set({ prompt: e.currentTarget.value })} />{/snippet}
      </Field>
      <div class="grid grid-cols-2 gap-3">
        {#if kind.type === "agent"}
          <Field label="Version" hint="Empty: the newest.">
            {#snippet children(f)}<Input id={f.id} type="number" min="1" value={kind.version ? String(kind.version) : ""} oninput={(e) => set({ version: e.currentTarget.value ? Number(e.currentTarget.value) : null })} />{/snippet}
          </Field>
        {/if}
        <Field label="Cost limit ($)" hint="Empty: the workflow's.">
          {#snippet children(f)}<Input id={f.id} type="number" min="0" step="0.05" value={usd(kind.max_cost_micros)} oninput={(e) => set({ max_cost_micros: micros(e.currentTarget.value) })} />{/snippet}
        </Field>
      </div>
      <Field label="Model" hint="Empty: the agent's own.">
        {#snippet children(f)}<Input id={f.id} mono value={kind.model ?? ""} placeholder="provider/model" oninput={(e) => set({ model: orNull(e.currentTarget.value) })} />{/snippet}
      </Field>
    {:else if kind.type === "condition"}
      <Field label="Is this true?" hint="Yes goes one way, no the other.">
        {#snippet children(f)}<Textarea id={f.id} mono rows={3} value={kind.expr} oninput={(e) => set({ expr: e.currentTarget.value })} />{/snippet}
      </Field>
      <p class="text-xs text-muted">Compare with <span class="mono">== != &lt; &lt;= &gt; &gt;=</span> and <span class="mono">contains</span>; join with <span class="mono">and</span>, <span class="mono">or</span>, <span class="mono">not</span>. Example: <span class="mono">severity &gt;= 3</span>.</p>
    {:else if kind.type === "loop"}
      <Field label="For each item of" hint="A list from an earlier step or the trigger. At most 8 items.">
        {#snippet children(f)}<Input id={f.id} mono value={kind.over} oninput={(e) => set({ over: e.currentTarget.value })} />{/snippet}
      </Field>
      <Field label="Agent">
        {#snippet children(f)}
          <Select id={f.id} value={kind.agent} onchange={(e) => set({ agent: e.currentTarget.value })}>
            {#each optionsOf(known.agents, kind.agent) as slug (slug)}<option value={slug}>{slug}</option>{/each}
          </Select>
        {/snippet}
      </Field>
      <Field label="What to ask for each" hint="{'{{'}item{'}}'} is the item, {'{{'}index{'}}'} its place.">
        {#snippet children(f)}<Textarea id={f.id} rows={4} value={kind.prompt} oninput={(e) => set({ prompt: e.currentTarget.value })} />{/snippet}
      </Field>
    {:else if kind.type === "wait"}
      <div>
        <p class="mb-1.5 text-sm font-medium">Waits</p>
        <Segmented label="Wait for or until" bind:value={() => kind.until ? "until" : "for", (v) => set(v === "until" ? { for_ms: null, until: "09:00" } : { for_ms: 10 * 60_000, until: null })} options={[{ value: "for", label: "For a while" }, { value: "until", label: "Until a time" }]} />
      </div>
      {#if kind.until}
        <Field label="Until" hint="The next time it is this time of day.">
          {#snippet children(f)}<Input id={f.id} type="time" value={kind.until ?? "09:00"} oninput={(e) => set({ until: e.currentTarget.value })} />{/snippet}
        </Field>
      {:else}
        {@render duration("For", kind.for_ms, (v) => set({ for_ms: v ?? 60_000 }), "10")}
      {/if}
    {:else if kind.type === "approval"}
      <Field label="What is being asked">
        {#snippet children(f)}<Textarea id={f.id} rows={3} value={kind.message} oninput={(e) => set({ message: e.currentTarget.value })} />{/snippet}
      </Field>
      <div>
        <p class="mb-1.5 text-sm font-medium">Show them</p>
        <div class="flex flex-wrap gap-1.5" role="group" aria-label="What the person sees">
          {#each showOptions(spec, step.id) as option (option.path)}<Chip selected={showing.has(option.path)} onclick={() => toggleShow(option.path)}>{option.label}</Chip>{/each}
        </div>
      </div>
      <div class="grid grid-cols-[1fr_auto] items-end gap-3">
        {@render duration("Time out after", kind.timeout_ms, (v) => set({ timeout_ms: v }))}
        <Field label="Then">
          {#snippet children(f)}
            <Select id={f.id} value={kind.then ?? "refuse"} class="w-28" onchange={(e) => set({ then: e.currentTarget.value })}>
              <option value="refuse">Refuse</option>
              <option value="continue">Continue</option>
            </Select>
          {/snippet}
        </Field>
      </div>
      <ToggleRow
        label="Remind in Slack"
        description={kind.remind ? `After ${ms(kind.remind.after_ms)}, in ${kind.remind.channel}` : "A message to a channel while this waits"}
        checked={Boolean(kind.remind)}
        onchange={(on) => set({ remind: on ? { after_ms: 30 * 60_000, connection: "slack", channel: "#on-call" } : null })}
      />
      {#if kind.remind}
        {@const remind = kind.remind}
        <div class="space-y-3 rounded-xl border border-line bg-panel-2 p-3">
          <Field label="Connection">
            {#snippet children(f)}<Input id={f.id} value={remind.connection} oninput={(e) => set({ remind: { ...remind, connection: e.currentTarget.value } })} />{/snippet}
          </Field>
          <Field label="Channel">
            {#snippet children(f)}<Input id={f.id} value={remind.channel} oninput={(e) => set({ remind: { ...remind, channel: e.currentTarget.value } })} />{/snippet}
          </Field>
          {@render duration("Remind after", remind.after_ms, (v) => set({ remind: { ...remind, after_ms: v ?? 30 * 60_000 } }), "30")}
          <p class="text-xs text-warn">A person confirms this reminder when publishing.</p>
        </div>
      {/if}
    {:else if kind.type === "question"}
      <Field label="Question">
        {#snippet children(f)}<Textarea id={f.id} rows={3} value={kind.question} oninput={(e) => set({ question: e.currentTarget.value })} />{/snippet}
      </Field>
      <Field label="Default answer" hint="Used when it times out and the step continues.">
        {#snippet children(f)}<Input id={f.id} value={kind.default ?? ""} oninput={(e) => set({ default: orNull(e.currentTarget.value) })} />{/snippet}
      </Field>
      <div class="grid grid-cols-[1fr_auto] items-end gap-3">
        {@render duration("Time out after", kind.timeout_ms, (v) => set({ timeout_ms: v }))}
        <Field label="Then">
          {#snippet children(f)}
            <Select id={f.id} value={kind.then ?? "refuse"} class="w-28" onchange={(e) => set({ then: e.currentTarget.value })}>
              <option value="refuse">Refuse</option>
              <option value="continue">Continue</option>
            </Select>
          {/snippet}
        </Field>
      </div>
    {:else if kind.type === "send"}
      <div>
        <p class="mb-1.5 text-sm font-medium">Where</p>
        <Segmented label="Service" bind:value={() => kind.service, (v) => set({ service: v })} options={[{ value: "slack", label: "Slack" }, { value: "discord", label: "Discord" }]} />
      </div>
      <Field label="Connection" hint="Empty: the only one of that service.">
        {#snippet children(f)}<Input id={f.id} value={kind.connection ?? ""} oninput={(e) => set({ connection: orNull(e.currentTarget.value) })} />{/snippet}
      </Field>
      <Field label="Channel">
        {#snippet children(f)}<Input id={f.id} value={kind.channel} oninput={(e) => set({ channel: e.currentTarget.value })} />{/snippet}
      </Field>
      <Field label="Message" hint="Use {'{{'}steps.<id>.output.text{'}}'} to send what an agent wrote.">
        {#snippet children(f)}<Textarea id={f.id} rows={4} value={kind.text} oninput={(e) => set({ text: e.currentTarget.value })} />{/snippet}
      </Field>
      {@render askRow(kind.ask)}
    {:else if kind.type === "http"}
      <div class="grid grid-cols-[6rem_1fr] gap-3">
        <Field label="Method">
          {#snippet children(f)}
            <Select id={f.id} value={kind.method} onchange={(e) => set({ method: e.currentTarget.value })}>
              {#each ["GET", "POST", "PUT", "PATCH", "DELETE"] as method (method)}<option value={method}>{method}</option>{/each}
            </Select>
          {/snippet}
        </Field>
        <Field label="Address">
          {#snippet children(f)}<Input id={f.id} mono value={kind.url} oninput={(e) => set({ url: e.currentTarget.value })} />{/snippet}
        </Field>
      </div>
      <Field label="Body" hint="Text, or JSON you write yourself.">
        {#snippet children(f)}<Textarea id={f.id} mono rows={4} value={kind.body ?? ""} oninput={(e) => set({ body: orNull(e.currentTarget.value) })} />{/snippet}
      </Field>
      {@render askRow(kind.ask)}
    {/if}

    <div>
      <p class="mb-1.5 text-sm font-medium">Leads to</p>
      {#if outgoing.length === 0}
        <p class="text-xs text-muted">Nothing yet. Drag from the dot under the step to another step, or choose one here.</p>
      {:else}
        <ul class="space-y-1.5">
          {#each outgoing as edge (edge.to)}
            <li class="flex items-center gap-2 rounded-lg border border-line bg-panel-2 py-1 pr-1 pl-2.5">
              {#if kind.type === "condition" && edge.label}
                <Segmented label="Branch to {titleOf(edge.to)}" bind:value={() => edge.label ?? "yes", (v) => onchange(relabel(spec, step.id, edge.to, v as "yes" | "no"))} options={[{ value: "yes", label: "yes" }, { value: "no", label: "no" }]} />
              {/if}
              <span class="min-w-0 flex-1 truncate text-sm">{titleOf(edge.to)}</span>
              <IconButton icon="x" label="Remove the arrow to {titleOf(edge.to)}" size="sm" onclick={() => onchange(disconnect(spec, step.id, edge.to))} />
            </li>
          {/each}
        </ul>
      {/if}
      {#if candidates.length > 0}
        <div class="mt-2 flex gap-2">
          <Select bind:value={target} aria-label="Add an arrow to" class="min-w-0 flex-1">
            <option value="">Add an arrow to…</option>
            {#each candidates as candidate (candidate.id)}<option value={candidate.id}>{candidate.title}</option>{/each}
          </Select>
          <Button size="sm" disabled={!target} onclick={addArrow}>Add</Button>
        </div>
      {/if}
    </div>

    {#if hasLive}
      <div class="border-t border-line-soft pt-4">
        <p class="text-2xs font-semibold tracking-wider text-faint uppercase">Last 5 runs of this step</p>
        {#if history && history.outcomes.length > 0}
          <div class="mt-2 flex items-center justify-between gap-3">
            <span class="flex h-5.5 items-end gap-0.5" role="img" aria-label="Last {history.outcomes.length} runs of this step: {history.outcomes.join(', ')}">
              {#each history.outcomes as outcome, i (i)}<i class="block h-full w-1.5 rounded-xs {OUTCOME_BAR[outcome] ?? 'bg-faint'}"></i>{/each}
            </span>
            <span class="text-xs text-muted">{history.median_wait_ms === null ? "no one waited" : `median wait ${ms(Math.max(history.median_wait_ms, 60_000))}`}</span>
          </div>
        {:else}
          <p class="mt-1.5 text-xs text-muted">This step has not run yet.</p>
        {/if}
      </div>
    {/if}
  </div>
{/if}
