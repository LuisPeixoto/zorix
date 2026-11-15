<script lang="ts">
  import { duration, isFinished, isLive, shortId, tokens } from "../format.ts";
  import { store } from "../stores/runs.svelte.ts";
  import ApprovalCard from "./ApprovalCard.svelte";
  import StatusBadge from "./StatusBadge.svelte";
  import Timeline from "./Timeline.svelte";

  const detail = $derived(store.detail);
  const run = $derived(detail?.execution);
  const pending = $derived(detail?.approvals.filter((a) => a.status === "pending") ?? []);
</script>

{#if run && detail}
  <div class="flex h-full flex-col overflow-y-auto">
    <header class="border-b border-line bg-panel px-6 py-4">
      <div class="flex items-start justify-between gap-4">
        <div class="min-w-0">
          <StatusBadge state={run.state} />
          <h2 class="mt-1 text-2xl font-semibold leading-tight break-words">{run.request.prompt}</h2>
          <p class="mt-1 font-mono text-xs text-faint">{run.id}</p>
        </div>
        <div class="flex shrink-0 gap-2">
          {#if run.state === "paused"}
            <button class="rounded-md bg-accent px-3 py-1.5 font-medium text-accent-fg" disabled={store.busy} onclick={() => store.resume()}>Resume</button>
          {/if}
          {#if isLive(run.state)}
            <button class="rounded-md border border-line px-3 py-1.5 hover:bg-hover" disabled={store.busy} onclick={() => store.pause()}>Pause</button>
          {/if}
          {#if !isFinished(run.state)}
            <button class="rounded-md border border-line px-3 py-1.5 text-err hover:bg-hover" disabled={store.busy} onclick={() => store.cancel()}>Cancel</button>
          {/if}
        </div>
      </div>

      <dl class="mt-4 grid grid-cols-2 gap-x-6 gap-y-2 sm:grid-cols-4">
        <div><dt class="text-xs text-faint">Model steps</dt><dd class="text-lg font-semibold">{detail.summary.model_calls}</dd></div>
        <div><dt class="text-xs text-faint">Tool calls</dt><dd class="text-lg font-semibold">{run.usage.tool_calls}</dd></div>
        <div><dt class="text-xs text-faint">Tokens</dt><dd class="text-lg font-semibold">{tokens(run.usage)}</dd></div>
        <div><dt class="text-xs text-faint" title="Time spent working; waiting for you is not counted">Work time</dt><dd class="text-lg font-semibold">{duration(run.active_ms)}</dd></div>
      </dl>
    </header>

    <div class="space-y-5 p-6">
      {#each pending as approval (approval.id)}
        <ApprovalCard {approval} />
      {/each}

      {#if run.result}
        <section class="rounded-lg border border-line bg-panel p-4" aria-label="Result">
          <h3 class="text-base font-semibold text-ok">Result</h3>
          <pre class="mt-2 font-sans whitespace-pre-wrap">{run.result.text}</pre>
        </section>
      {/if}
      {#if run.failure}
        <section class="rounded-lg border border-err/60 bg-err/10 p-4" aria-label="Failure">
          <h3 class="text-base font-semibold text-err">It did not finish</h3>
          <p class="mt-1">{run.failure.message}</p>
        </section>
      {/if}
      {#if run.state === "cancelled"}
        <p class="text-muted">Cancelled{run.cancel_reason ? `: ${run.cancel_reason}` : "."}</p>
      {/if}

      <section aria-label="Timeline">
        <h3 class="mb-2 text-base font-semibold">What happened</h3>
        <Timeline events={store.events} />
      </section>

      {#if detail.tool_calls.length}
        <section aria-label="Tool calls">
          <h3 class="mb-2 text-base font-semibold">Tools</h3>
          <ul class="divide-y divide-line rounded-lg border border-line bg-panel">
            {#each detail.tool_calls as call (call.call_id)}
              <li class="flex items-baseline gap-3 px-3 py-2">
                <span class="font-mono text-xs">{call.tool}</span>
                <span class="min-w-0 flex-1 truncate font-mono text-xs text-muted">{JSON.stringify(call.arguments)}</span>
                <span class="text-xs text-muted">{call.status.replace("_", " ")}</span>
              </li>
            {/each}
          </ul>
        </section>
      {/if}
      <p class="text-xs text-faint">Run {shortId(run.id)} · model {run.request.model}</p>
    </div>
  </div>
{:else}
  <p class="p-6 text-muted">Loading…</p>
{/if}
