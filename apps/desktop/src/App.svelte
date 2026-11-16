<script lang="ts">
  import { onMount } from "svelte";

  import { ago, shorten } from "./lib/format.ts";
  import { store } from "./lib/stores/runs.svelte.ts";
  import Providers from "./lib/components/Providers.svelte";
  import NewRun from "./lib/components/NewRun.svelte";
  import RunView from "./lib/components/RunView.svelte";
  import StatusBadge from "./lib/components/StatusBadge.svelte";

  onMount(() => void store.init());
</script>

<div class="flex h-full">
  <aside class="flex w-72 shrink-0 flex-col border-r border-line bg-panel">
    <div class="flex items-center justify-between px-4 py-4">
      <h1 class="text-xl font-bold tracking-tight">ZORIX</h1>
      <button class="rounded-md bg-accent px-3 py-1 font-medium text-accent-fg" onclick={() => { store.showRuns(); store.showForm(); }}>New run</button>
    </div>
    <nav class="min-h-0 flex-1 overflow-y-auto px-2 pb-2" aria-label="Runs">
      {#each store.runs as run (run.id)}
        <button
          class="mb-0.5 block w-full rounded-md px-3 py-2 text-left hover:bg-hover {store.selectedId === run.id ? 'bg-accent-soft' : ''}"
          onclick={() => { store.showRuns(); void store.select(run.id); }}
        >
          <span class="block truncate font-medium">{shorten(run.request.prompt, 60)}</span>
          <span class="mt-0.5 flex items-center justify-between text-xs text-muted">
            <StatusBadge state={run.state} />
            <span>{ago(run.created_at, store.now)}</span>
          </span>
        </button>
      {:else}
        <p class="px-3 py-2 text-muted">No runs yet. Start one.</p>
      {/each}
    </nav>
    <div class="border-t border-line px-2 py-2">
      <button class="block w-full rounded-md px-3 py-1.5 text-left hover:bg-hover {store.view === 'providers' ? 'bg-accent-soft' : ''}" onclick={() => store.showProviders()}>Providers &amp; keys</button>
    </div>
    <footer class="border-t border-line px-4 py-2 text-xs text-faint">
      <p>v{store.info?.version ?? ""}</p>
      <p class="truncate" title={store.info?.data_dir}>{store.info?.data_dir ?? ""}</p>
    </footer>
  </aside>

  <main class="min-w-0 flex-1">
    {#if store.info?.error}
      <div class="mx-auto max-w-xl p-10" role="alert">
        <h2 class="text-2xl font-semibold text-err">ZORIX could not start</h2>
        <p class="mt-2">{store.info.error}</p>
      </div>
    {:else if store.view === "providers"}
      <Providers />
    {:else if store.selectedId}
      {#if store.error}<p class="border-b border-line bg-err/10 px-6 py-2 text-err" role="alert">{store.error}</p>{/if}
      <RunView />
    {:else}
      <NewRun />
    {/if}
  </main>
</div>
