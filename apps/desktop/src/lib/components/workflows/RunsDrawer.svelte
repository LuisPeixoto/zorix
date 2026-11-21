<script lang="ts">
  import { type WfRun, workflowRuns } from "../../api.ts";
  import { ago } from "../../format.ts";
  import { store } from "../../stores/runs.svelte.ts";
  import { Badge, Drawer, EmptyState } from "../../ui/index.ts";
  import { RUN_STATE_LABEL, ms } from "../../workflow.ts";

  let { open = $bindable(false), id }: { open?: boolean; id: string } = $props();

  let runs = $state<WfRun[]>([]);
  let loaded = $state(false);

  $effect(() => {
    if (!open) return;
    loaded = false;
    void workflowRuns(id, 40).then((r) => {
      runs = r;
      loaded = true;
    });
  });

  const TONE = { running: "info", waiting: "warn", completed: "ok", failed: "err", cancelled: "muted" } as const;
  const usd = (micros: number) => `$${(micros / 1_000_000).toFixed(2)}`;
</script>

<Drawer bind:open title="Runs" subtitle="The runs of this workflow, newest first. Open one to see every step." icon="history">
  {#if loaded && runs.length === 0}
    <EmptyState icon="play" title="No runs yet" text="Press Test run to try the draft, or publish it and turn it on." />
  {:else}
    <ul class="divide-y divide-line-soft overflow-hidden rounded-2xl border border-line bg-panel">
      {#each runs as run (run.id)}
        <li>
          <button type="button" class="flex w-full items-center gap-3 px-4 py-3 text-left transition duration-150 hover:bg-hover" onclick={() => store.showWorkflows("run", run.workflow, run.id)}>
            <span class="min-w-0 flex-1">
              <span class="block truncate text-base font-medium">{run.fired.label}{run.test ? " · test" : ""}</span>
              <span class="mt-0.5 block text-xs text-muted">{run.version ? `v${run.version}` : "draft"} · {ago(run.started_at, store.now)} · {run.finished_at ? ms(Math.max(run.finished_at - run.started_at, 60_000)) : "going"} · <span class="mono">{usd(run.cost_micros)}</span></span>
            </span>
            <Badge tone={TONE[run.state]} dot={run.state === "running" || run.state === "waiting"} live={run.state === "running" || run.state === "waiting"}>{RUN_STATE_LABEL[run.state]}</Badge>
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</Drawer>
