<script lang="ts">
  import { type WfRunDetail, errorText, workflowCancel, workflowRun } from "../../api.ts";
  import { ago, json, shortId } from "../../format.ts";
  import { store } from "../../stores/runs.svelte.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { Badge, Button, Callout, Card, CodeBlock, Icon, KeyValue, Markdown, TriggerChip, WorkflowCanvas } from "../../ui/index.ts";
  import { NODE_TEXT, NODE_TINT, RUN_STATE_LABEL, chipKind, kindInfo, kindLine, ms, nodeState } from "../../workflow.ts";
  import WorkflowWaitCard from "./WorkflowWaitCard.svelte";

  let { runId }: { runId: string } = $props();

  let detail = $state<WfRunDetail | null>(null);
  let error = $state("");
  let selected = $state<string | null>(null);
  let picked = false;
  let busy = $state(false);

  async function load() {
    try {
      detail = await workflowRun(runId);
      error = "";
      if (!picked) selected = firstToLookAt();
    } catch (e) {
      error = errorText(e);
    }
  }

  // The run is alive: steps end, waits are answered, timers elapse. Looking at it also moves it along.
  $effect(() => {
    void runId;
    picked = false;
    selected = null;
    void load();
    const timer = setInterval(() => {
      if (!detail?.run || !["completed", "failed", "cancelled"].includes(detail.run.state)) void load();
    }, 1500);
    return () => clearInterval(timer);
  });

  const run = $derived(detail?.run);
  const finished = $derived(run ? ["completed", "failed", "cancelled"].includes(run.state) : false);
  const TONE = { running: "info", waiting: "warn", completed: "ok", failed: "err", cancelled: "muted" } as const;
  const usd = (micros: number) => `$${(micros / 1_000_000).toFixed(2)}`;

  function firstToLookAt(): string | null {
    if (!detail) return null;
    const { run, waits } = detail;
    const pending = waits.find((w) => w.status === "pending" && w.kind !== "timer");
    if (pending) return pending.step;
    const failed = run.spec.steps.find((s) => run.steps[s.id]?.state === "failed");
    if (failed) return failed.id;
    const going = run.spec.steps.find((s) => ["running", "waiting"].includes(run.steps[s.id]?.state));
    if (going) return going.id;
    return [...run.spec.steps].reverse().find((s) => run.steps[s.id]?.state === "done")?.id ?? null;
  }

  const step = $derived(run && selected ? run.spec.steps.find((s) => s.id === selected) : undefined);
  const record = $derived(run && step ? run.steps[step.id] : undefined);
  const wait = $derived(step && detail ? detail.waits.find((w) => w.step === step.id && w.status === "pending" && w.kind !== "timer") : undefined);
  const output = $derived((record?.output ?? null) as Record<string, unknown> | null);
  const outputText = $derived(output && typeof output.text === "string" ? output.text : "");
  const triggerChip = $derived(run ? run.spec.steps.find((s) => s.id === run.fired.step) : undefined);

  const STATE_WORD = { pending: "Not reached", running: "Running", waiting: "Waiting", done: "Done", failed: "Failed", skipped: "Skipped" } as const;
  const STATE_TONE = { pending: "muted", running: "info", waiting: "warn", done: "ok", failed: "err", skipped: "muted" } as const;

  async function cancel() {
    busy = true;
    try {
      await workflowCancel(runId);
      toast.ok("Run cancelled.");
      await load();
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      busy = false;
    }
  }

  const duration = (start?: number | null, end?: number | null) => (start && end ? ms(Math.max(end - start, 1000)).replace(/^0 min$/, "<1 min") : "");
</script>

{#if error && !detail}
  <div class="p-6"><Callout tone="err" title="This run could not be opened">{error}</Callout></div>
{:else if run}
  <div class="grid h-full grid-rows-[auto_minmax(0,1fr)]">
    <header class="flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-line bg-panel px-5 py-3">
      <div class="min-w-0">
        <nav aria-label="Breadcrumb" class="mb-0.5 flex items-center gap-1.5 text-xs text-muted">
          <button type="button" class="inline-flex items-center gap-1.5 rounded outline-none hover:text-fg focus-visible:underline" onclick={() => store.showWorkflows()}><Icon name="workflow" size={13} />Workflows</button>
          <Icon name="chevron-right" size={12} />
          <button type="button" class="rounded outline-none hover:text-fg focus-visible:underline" onclick={() => store.showWorkflows("editor", run.workflow, null, "runs")}>{run.spec.name}</button>
          <Icon name="chevron-right" size={12} />
          <span>Runs</span>
        </nav>
        <div class="flex flex-wrap items-center gap-2">
          <h1 class="text-xl font-semibold">Run <span class="mono">#{shortId(run.id)}</span></h1>
          <Badge tone={TONE[run.state]} dot={run.state === "running" || run.state === "waiting"} live={run.state === "running" || run.state === "waiting"}>{RUN_STATE_LABEL[run.state]}</Badge>
          <Badge tone="muted" variant="outline">{run.version ? `v${run.version}` : "draft"}</Badge>
          {#if run.test}<Badge tone="accent">Test run</Badge>{/if}
        </div>
      </div>
      <div class="flex flex-wrap items-center gap-3">
        {#if triggerChip}<TriggerChip kind={chipKind(triggerChip.kind.type)} label={run.fired.label} />{/if}
        <span class="text-xs text-muted">Started {ago(run.started_at, store.now)} · <span class="mono tnum">{usd(run.cost_micros)}</span></span>
      </div>
      <div class="ml-auto flex items-center gap-2">
        <Button variant="ghost" icon="edit" onclick={() => store.showWorkflows("editor", run.workflow)}>Open in editor</Button>
        {#if !finished}<Button icon="stop" disabled={busy} onclick={() => void cancel()}>Cancel run</Button>{/if}
      </div>
    </header>

    <div class="grid min-h-0 grid-cols-[minmax(0,1fr)_420px]">
      <WorkflowCanvas
        spec={run.spec}
        {run}
        readonly
        bind:selected
        class="rounded-none border-0"
      />

      <aside class="min-h-0 space-y-4 overflow-y-auto border-l border-line bg-panel p-4" aria-label="Selected step">
        {#if run.error}
          <Callout tone="err" title="This run failed">{run.error}</Callout>
        {/if}
        {#if step && record}
          {@const info = kindInfo(step.kind.type)}
          <div class="flex items-center gap-2.5">
            <span class="grid size-8 shrink-0 place-items-center rounded-lg {NODE_TINT[info.cls]} {NODE_TEXT[info.cls]}"><Icon name={info.icon} size={16} /></span>
            <div class="min-w-0 flex-1">
              <p class="truncate text-md font-semibold">{step.title}</p>
              <p class="truncate text-xs text-muted">{kindLine(step)}</p>
            </div>
            <Badge tone={STATE_TONE[record.state]}>{STATE_WORD[record.state]}</Badge>
          </div>

          {#if wait}
            <WorkflowWaitCard {wait} spec={run.spec} onanswered={() => void load()} />
          {:else}
            {#if record.error && !run.error?.includes(record.error)}<Callout tone="err" title="Why it failed">{record.error}</Callout>{/if}
            {#if record.note && record.state !== "failed"}<p class="text-sm text-muted">{record.note}</p>{/if}
            {#if record.execution}
              <Button icon="bot" onclick={() => void store.select(record.execution as string)}>Open the run</Button>
            {/if}
            {#if outputText}
              <Card title="What it said" icon="message" padding="sm"><Markdown text={outputText} /></Card>
            {:else if output && Object.keys(output).length > 0 && record.state === "done"}
              <CodeBlock code={json(output)} language="json" maxHeight={220} label="What it produced" />
            {/if}
          {/if}
        {:else}
          <p class="text-sm text-muted">Select a step to see what it did.</p>
        {/if}

        <Card title="Steps of this run" icon="list" padding="none">
          <ul class="divide-y divide-line-soft">
            {#each run.spec.steps as s (s.id)}
              {@const r = run.steps[s.id]}
              {@const k = kindInfo(s.kind.type)}
              <li>
                <button type="button" class="flex w-full items-center gap-2.5 px-3.5 py-2.5 text-left transition duration-150 outline-none hover:bg-hover focus-visible:bg-hover {selected === s.id ? 'bg-select' : ''}" aria-current={selected === s.id} onclick={() => { selected = s.id; picked = true; }}>
                  <span class="grid size-5.5 shrink-0 place-items-center rounded-md {NODE_TINT[k.cls]} {NODE_TEXT[k.cls]}"><Icon name={k.icon} size={12} /></span>
                  <span class="min-w-0 flex-1">
                    <span class="block truncate text-sm font-medium">{s.title}</span>
                    <span class="block truncate text-xs text-muted">{r?.note || r?.error || kindLine(s)}</span>
                  </span>
                  {#if r && r.state !== "pending"}
                    <Badge tone={STATE_TONE[nodeState(r.state) === "idle" ? "pending" : r.state]} dot={r.state === "running" || r.state === "waiting"} live={r.state === "running" || r.state === "waiting"}>{STATE_WORD[r.state]}</Badge>
                    {#if r.state === "done" && r.started_at && r.finished_at}<span class="tnum w-14 text-right text-xs text-faint">{duration(r.started_at, r.finished_at)}</span>{/if}
                  {:else}
                    <span class="text-xs text-faint">Not reached</span>
                  {/if}
                </button>
              </li>
            {/each}
          </ul>
        </Card>

        <KeyValue
          items={[
            { key: "Started by", value: run.fired.label },
            { key: "Version", value: run.version ? `v${run.version}` : "the draft (a test)" },
            { key: "Cost", value: usd(run.cost_micros) },
            { key: "Budget", value: run.spec.budget_micros ? `${usd(run.cost_micros)} of ${usd(run.spec.budget_micros)}` : "No limit" },
          ]}
        />
      </aside>
    </div>
  </div>
{/if}
