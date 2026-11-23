<script lang="ts">
  import { t } from "../i18n/index.ts";
  import { ago, shorten, shortId } from "../format.ts";
  import { inFilter, type RunFilter, runningCount, usd } from "../shell.ts";
  import { store } from "../stores/runs.svelte.ts";
  import { Badge, Button, EmptyState, Icon, PageHeader, Segmented, Stat } from "../ui/index.ts";
  import StatusBadge from "./StatusBadge.svelte";

  let filter = $state<RunFilter>("all");

  const shown = $derived(store.runs.filter((r) => inFilter(r, filter)));
  const count = (f: RunFilter) => store.runs.filter((r) => inFilter(r, f)).length;
  const waiting = $derived(count("waiting"));
  const spent = $derived(store.runs.reduce((sum, r) => sum + r.usage.cost_micros, 0));
</script>

<div class="h-full overflow-y-auto">
  <div class="mx-auto max-w-[1100px] p-6">
    <PageHeader title={t("common.runs")} subtitle={t("runsPage.everythingYouAskedZorix")} glow>
      {#snippet actions()}
        <Button variant="primary" icon="plus" onclick={() => { store.continuing = null; store.showForm(); }}>{t("common.newRun")}</Button>
      {/snippet}
    </PageHeader>

    {#if store.runs.length === 0}
      <EmptyState icon="history" title={t("common.noRunsYet")} text={t("runsPage.sayWhatYouWant")}>
        {#snippet actions()}<Button variant="primary" icon="plus" onclick={() => store.showForm()}>{t("common.newRun")}</Button>{/snippet}
      </EmptyState>
    {:else}
      <div class="mb-5 grid gap-4 sm:grid-cols-3">
        <Stat icon="play" label={t("runsPage.runningNow")} value={runningCount(store.runs)} detail="{store.runs.length} runs listed" />
        <Stat icon="shield" label={t("common.waitingForYou")} value={waiting} detail={waiting === 1 ? "one run needs a decision" : "runs that need a decision"} />
        <Stat icon="dollar" label={t("runsPage.costOfTheseRuns")} value={usd(spent)} detail="including what agents started" />
      </div>

      <div class="mb-3 flex flex-wrap items-center justify-between gap-3">
        <Segmented
          label={t("common.show")}
          bind:value={filter}
          options={[
            { value: "all", label: `All ${store.runs.length}` },
            { value: "running", label: `Running ${count("running")}` },
            { value: "waiting", label: `Waiting ${waiting}` },
            { value: "finished", label: `Finished ${count("finished")}` },
          ]}
        />
      </div>

      {#if shown.length === 0}
        <p class="rounded-2xl border border-dashed border-line-strong bg-panel px-6 py-8 text-center text-base text-muted">{t("runsPage.noRunsHere")}</p>
      {:else}
        <ul aria-label={t("common.runs")} class="divide-y divide-line-soft overflow-hidden rounded-2xl border border-line bg-panel shadow-card">
          {#each shown as run (run.id)}
            <li>
              <button type="button" onclick={() => void store.select(run.id)} class="flex w-full items-center gap-3 px-4 py-3 text-left transition duration-150 hover:bg-hover">
                <span class="min-w-0 flex-1">
                  <span class="block truncate text-md font-medium">{shorten(run.request.prompt, 110)}</span>
                  <span class="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted">
                    <span class="mono">{shortId(run.id)}</span>
                    {#if run.agent}
                      <span class="inline-flex items-center gap-1"><Icon name={run.agent.team ? "users" : "bot"} size={12} />{run.agent.team ?? run.agent.slug} v{run.agent.version}</span>
                    {/if}
                    {#if store.subRuns[run.id]}
                      <span class="inline-flex items-center gap-1"><Icon name="branch" size={12} />{store.subRuns[run.id]} sub-run{store.subRuns[run.id] === 1 ? "" : "s"}</span>
                    {/if}
                    <span class="mono tnum">{usd(run.usage.cost_micros)}</span>
                  </span>
                </span>
                {#if store.needsYou[run.id]}<Badge tone="warn" dot>{t("runsPage.needsYouInThe")}</Badge>{/if}
                <StatusBadge state={run.state} />
                <span class="tnum w-20 shrink-0 text-right text-xs text-muted">{ago(run.created_at, store.now)}</span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    {/if}
  </div>
</div>
