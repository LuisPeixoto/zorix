<script lang="ts">
  import { t as tr } from "../../i18n/index.ts";
  import { onDestroy } from "svelte";

  import {
    type AgentDetail,
    type CompareRow,
    type EvalCase,
    type EvalRun,
    type StoredCase,
    errorText,
    evalCancel,
    evalCases,
    evalCompare,
    evalPutCase,
    evalRemoveCase,
    evalRunning,
    evalRuns,
    evalStart,
  } from "../../api.ts";
  import { ago, shorten } from "../../format.ts";
  import { store } from "../../stores/runs.svelte.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { Badge, Button, Callout, Card, EmptyState, Icon, IconButton, Input, ScoreMatrix, Switch } from "../../ui/index.ts";
  import CaseEditor from "./CaseEditor.svelte";

  let { detail }: { detail: AgentDetail } = $props();
  const slug = $derived(detail.agent.slug);

  let cases = $state<StoredCase[]>([]);
  let runs = $state<EvalRun[]>([]);
  let running = $state(false);
  let models = $state("");
  let offline = $state(false);
  let selected = $state<{ caseName: string; runId: string } | null>(null);
  let change = $state<CompareRow[]>([]);
  let error = $state<string | null>(null);
  let editing = $state<EvalCase | null>(null);
  let editorOpen = $state(false);
  let timer: ReturnType<typeof setTimeout> | null = null;
  let alive = true;

  const shown = $derived(runs.slice(0, 5));
  const matrixRuns = $derived(shown.map((r) => ({ id: r.id, label: `v${r.version} · ${r.model}`, sub: r.status === "running" ? tr("agents.agentEvals.running") : ago(r.started_at, store.now) })));
  const names = $derived([...new Set([...cases.map((c) => c.case.name), ...shown.flatMap((r) => r.results.map((x) => x.case))])]);
  const cell = (caseName: string, runId: string) => {
    const result = runs.find((r) => r.id === runId)?.results.find((x) => x.case === caseName);
    if (!result) return { state: "none" as const };
    return { state: result.passed ? ("pass" as const) : ("fail" as const), note: result.error ?? result.checks.filter((c) => !c.passed).map((c) => c.check).join("; ") };
  };
  const result = $derived(selected ? runs.find((r) => r.id === selected!.runId)?.results.find((x) => x.case === selected!.caseName) : undefined);
  const counts = $derived({
    fixed: change.filter((c) => c.change === "Fixed").length,
    broke: change.filter((c) => c.change === "Broke").length,
  });

  async function load() {
    try {
      [cases, runs, running] = await Promise.all([evalCases(slug), evalRuns(slug), evalRunning(slug)]);
      error = null;
      if (runs.length >= 2) change = await evalCompare(runs[1].id, runs[0].id);
      else change = [];
    } catch (e) {
      error = errorText(e);
    }
    if (alive && (running || runs.some((r) => r.status === "running"))) {
      if (timer) clearTimeout(timer);
      timer = setTimeout(load, 1000);
    }
  }
  $effect(() => {
    void slug;
    void load();
  });
  onDestroy(() => {
    alive = false;
    if (timer) clearTimeout(timer);
  });

  async function run() {
    try {
      await evalStart({ slug, models: models.split(",").map((m) => m.trim()).filter(Boolean), offline });
      running = true;
      await load();
    } catch (e) {
      toast.err(errorText(e));
    }
  }

  async function stop() {
    try {
      await evalCancel(slug);
    } catch (e) {
      toast.err(errorText(e));
    }
  }

  async function saveCase(c: EvalCase): Promise<string | null> {
    try {
      await evalPutCase(slug, c);
      await load();
      return null;
    } catch (e) {
      return errorText(e);
    }
  }

  async function removeCase(name: string) {
    try {
      await evalRemoveCase(slug, name);
      await load();
    } catch (e) {
      toast.err(errorText(e));
    }
  }

  function openRun(id: string) {
    store.showRuns();
    void store.select(id);
  }
</script>

<div class="h-full overflow-y-auto">
  <div class="mx-auto flex max-w-5xl flex-col gap-6 p-8">
    {#if error}<Callout tone="err" title={tr("agents.agentEvals.evaluationsCouldNotBe")}>{error}</Callout>{/if}

    <section aria-labelledby="results-title">
      <div class="mb-3 flex flex-wrap items-center gap-3">
        <h2 id="results-title" class="text-lg font-semibold">{tr("agents.agentEvals.results")}</h2>
        {#if counts.fixed || counts.broke}
          <span class="flex gap-1.5 text-xs"><Badge tone="ok">{counts.fixed} fixed</Badge><Badge tone={counts.broke ? "err" : "muted"}>{counts.broke} broke</Badge><span class="self-center text-faint">{tr("agents.agentEvals.sinceTheRunBefore")}</span></span>
        {/if}
        <span class="flex-1"></span>
        <Input class="w-64" size="sm" bind:value={models} placeholder={tr("agents.agentEvals.otherModelsToTry")} aria-label={tr("agents.agentEvals.modelsToTry")} disabled={running} />
        <Switch bind:checked={offline} disabled={running}>{tr("agents.agentEvals.offline")}</Switch>
        {#if running}
          <Button icon="stop" onclick={stop}>{tr("common.stop")}</Button>
        {:else}
          <Button variant="primary" icon="play" disabled={cases.length === 0} onclick={run}>{tr("agents.agentEvals.runEvaluation")}</Button>
        {/if}
      </div>

      {#if cases.length === 0}
        <EmptyState icon="flask" title={tr("agents.agentEvals.noCasesYet")} text={tr("agents.agentEvals.aCaseIsSomething")} />
      {:else if shown.length === 0}
        <EmptyState icon="flask" title={tr("agents.agentEvals.notRunYet")} text={tr("agents.agentEvals.runTheEvaluationTo")} />
      {:else}
        <ScoreMatrix cases={names} runs={matrixRuns} {cell} {selected} onselect={(caseName, runId) => (selected = { caseName, runId })} />
        {#if result && selected}
          <Card class="mt-4" title={selected.caseName} icon={result.passed ? "check-circle" : "x-circle"}>
            {#snippet actions()}{#if result.execution}<Button size="sm" icon="external" onclick={() => openRun(result.execution!)}>{tr("common.openTheRun")}</Button>{/if}{/snippet}
            <p class="mb-3 text-sm text-muted">{tr("agents.agentEvals.resultSummary", {
              steps: tr("agents.agentEvals.steps", { count: result.steps }),
              tools: tr("agents.agentEvals.toolCalls", { count: result.tool_calls }),
              state: result.state.replaceAll("_", " "),
            })}</p>
            {#if result.error}<p class="mb-2 text-sm text-err" role="alert">{result.error}</p>{/if}
            <ul class="space-y-1.5">
              {#each result.checks as c (c.check)}
                <li class="flex items-start gap-2 text-sm"><Icon name={c.passed ? "check" : "x"} size={14} stroke={2.4} class="mt-0.5 shrink-0 {c.passed ? 'text-ok' : 'text-err'}" /><span><span class="font-medium">{c.check}</span>{#if !c.passed}<span class="text-muted"> — {shorten(c.detail, 200)}</span>{/if}</span></li>
              {/each}
            </ul>
            {#if result.judge}
              <div class="mt-3 rounded-xl border border-line bg-panel-2 p-3 text-sm"><p class="font-medium">{tr("agents.agentEvals.graderOutOf5", { score: result.judge.score })} <span class="font-normal text-faint">· {result.judge.model}</span></p><p class="mt-1 text-muted">{result.judge.reasons}</p></div>
            {/if}
          </Card>
        {/if}
      {/if}
    </section>

    <section aria-labelledby="cases-title">
      <div class="mb-3 flex items-center gap-3">
        <h2 id="cases-title" class="flex-1 text-lg font-semibold">{tr("agents.agentEvals.cases")} <span class="text-sm font-normal text-faint">{cases.length}</span></h2>
        <Button size="sm" icon="plus" onclick={() => { editing = null; editorOpen = true; }}>{tr("agents.agentEvals.newCase")}</Button>
      </div>
      <div class="space-y-2">
        {#each cases as c (c.id)}
          <div class="raised flex items-start gap-3 rounded-xl border border-line bg-panel px-3.5 py-3">
            <div class="min-w-0 flex-1">
              <p class="mono text-md font-medium">{c.case.name}</p>
              <p class="mt-0.5 line-clamp-2 text-sm text-muted">{c.case.prompt}</p>
              <p class="mt-1.5 flex flex-wrap items-center gap-1.5 text-xs text-faint">{c.case.checks.length} check{c.case.checks.length === 1 ? "" : "s"}{#if c.case.judge}<Badge tone="accent">graded</Badge>{/if}{#each c.case.tags as t (t)}<Badge tone="muted">{t}</Badge>{/each}</p>
            </div>
            <IconButton icon="edit" label={tr("agents.agentEvals.edit", { name: c.case.name })} onclick={() => { editing = c.case; editorOpen = true; }} />
            <IconButton icon="trash" label={tr("agents.agentEvals.delete", { name: c.case.name })} onclick={() => removeCase(c.case.name)} />
          </div>
        {/each}
      </div>
    </section>
  </div>
</div>

<CaseEditor bind:open={editorOpen} initial={editing} onsave={saveCase} />
