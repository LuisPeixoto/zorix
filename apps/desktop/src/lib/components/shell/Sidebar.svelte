<script lang="ts">
  import { ago, shorten } from "../../format.ts";
  import { store } from "../../stores/runs.svelte.ts";
  import { Button } from "../../ui/index.ts";
  import StatusBadge from "../StatusBadge.svelte";
  import NavItem from "./NavItem.svelte";
</script>

<aside class="flex w-64 shrink-0 flex-col border-r border-line bg-panel-2">
  <div class="p-3">
    <Button variant="primary" icon="plus" class="w-full" onclick={() => { store.showRuns(); store.showForm(); }}>New run</Button>
  </div>

  <p class="px-5 pt-2 pb-1.5 text-2xs font-semibold tracking-wider text-faint uppercase">Runs</p>
  <nav class="min-h-0 flex-1 overflow-y-auto px-2 pb-2" aria-label="Runs">
    {#each store.runs as run (run.id)}
      <button
        type="button"
        class="mb-0.5 block w-full rounded-lg px-2.5 py-2 text-left transition duration-150 hover:bg-hover {store.view === 'runs' && store.selectedId === run.id ? 'bg-accent-soft' : ''}"
        onclick={() => { store.showRuns(); void store.select(run.id); }}
      >
        <span class="block truncate text-md font-medium">{shorten(run.request.prompt, 60)}</span>
        <span class="mt-0.5 flex items-center justify-between text-xs text-muted">
          <StatusBadge state={run.state} />
          <span>{ago(run.created_at, store.now)}</span>
        </span>
      </button>
    {:else}
      <p class="px-2.5 py-2 text-base text-muted">No runs yet. Start one.</p>
    {/each}
  </nav>

  <div class="border-t border-line px-2 py-2">
    <NavItem icon="key" active={store.view === "providers"} onclick={() => store.showProviders()}>Providers &amp; keys</NavItem>
  </div>
  <footer class="border-t border-line px-4 py-2.5 text-xs text-faint">
    <p class="mono">v{store.info?.version ?? ""}</p>
    <p class="truncate" title={store.info?.data_dir}>{store.info?.data_dir ?? ""}</p>
  </footer>
</aside>
