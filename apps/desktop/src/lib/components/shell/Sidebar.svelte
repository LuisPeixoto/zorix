<script lang="ts">
  import { ago, shorten } from "../../format.ts";
  import { agents } from "../../stores/agents.svelte.ts";
  import { integrations } from "../../stores/integrations.svelte.ts";
  import { store } from "../../stores/runs.svelte.ts";
  import { Button, Icon } from "../../ui/index.ts";
  import StatusBadge from "../StatusBadge.svelte";
  import NavItem from "./NavItem.svelte";

  const waiting = $derived(store.runs.filter((r) => r.state === "waiting_for_approval" || store.needsYou[r.id]).length);
</script>

<aside class="flex w-64 shrink-0 flex-col border-r border-line bg-panel-2">
  <div class="p-3">
    <Button variant="primary" icon="plus" class="w-full" onclick={() => { store.continuing = null; store.showRuns(); store.showForm(); }}>New run</Button>
  </div>

  <div class="flex items-center justify-between px-5 pt-2 pb-1.5">
    <p class="text-2xs font-semibold tracking-wider text-faint uppercase">Runs</p>
    {#if waiting > 0}<span class="tnum rounded-full bg-warn/15 px-1.5 text-2xs font-medium text-warn" title="Waiting for you">{waiting} waiting</span>{/if}
  </div>
  <nav class="min-h-0 flex-1 overflow-y-auto px-2 pb-2" aria-label="Runs">
    {#each store.runs as run (run.id)}
      <button
        type="button"
        class="mb-0.5 block w-full rounded-lg px-2.5 py-2 text-left transition duration-150 hover:bg-hover {store.view === 'runs' && store.selectedId === run.id ? 'bg-accent-soft' : ''}"
        onclick={() => { store.showRuns(); void store.select(run.id); }}
      >
        <span class="flex items-center gap-1.5 truncate text-md font-medium">{#if run.agent}<Icon name="bot" size={13} class="shrink-0 text-accent" />{/if}<span class="truncate">{shorten(run.request.prompt, 60)}</span></span>
        <span class="mt-0.5 flex items-center justify-between text-xs text-muted">
          <span class="flex items-center gap-1.5"><StatusBadge state={run.state} />{#if store.subRuns[run.id]}<span class="tnum inline-flex items-center gap-0.5 rounded-full bg-hover px-1.5 text-2xs whitespace-nowrap text-muted" title="{store.subRuns[run.id]} sub-run{store.subRuns[run.id] === 1 ? '' : 's'} started by agents for this run"><Icon name="branch" size={10} />{store.subRuns[run.id]}</span>{/if}</span>
          <span class="tnum whitespace-nowrap">{ago(run.created_at, store.now)}</span>
        </span>
      </button>
    {:else}
      <p class="px-2.5 py-2 text-base text-muted">No runs yet. Start one.</p>
    {/each}
  </nav>

  <div class="border-t border-line-soft px-2 pt-2 pb-1">
    <p class="px-2.5 pb-1 text-2xs font-semibold tracking-wider text-faint uppercase">Workspace</p>
    <NavItem icon="bot" active={store.view === "agents"} count={agents.cards.length} onclick={() => store.showAgents()}>Agents</NavItem>
    <NavItem icon="book" active={store.view === "memory"} onclick={() => store.showMemory()}>Memory</NavItem>
    <NavItem icon="plug" active={store.view === "integrations"} attention={integrations.attention > 0} count={integrations.servers.filter((s) => s.enabled).length} onclick={() => store.showIntegrations()}>Integrations</NavItem>
    <NavItem icon="key" active={store.view === "providers"} onclick={() => store.showProviders()}>Providers &amp; keys</NavItem>
    {#if import.meta.env.DEV}
      <NavItem icon="layers" active={store.view === "design"} onclick={() => store.showDesign()}>Design system</NavItem>
    {/if}
  </div>
  <footer class="border-t border-line-soft px-4 py-2.5 text-xs text-faint">
    <p class="mono">v{store.info?.version ?? ""}</p>
    <p class="truncate" title={store.info?.data_dir}>{store.info?.data_dir ?? ""}</p>
  </footer>
</aside>
