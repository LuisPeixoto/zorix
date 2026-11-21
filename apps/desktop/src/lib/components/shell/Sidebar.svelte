<script lang="ts">
  import { agents } from "../../stores/agents.svelte.ts";
  import { inbox } from "../../stores/inbox.svelte.ts";
  import { integrations } from "../../stores/integrations.svelte.ts";
  import { store } from "../../stores/runs.svelte.ts";
  import { runningCount, spendToday, usd } from "../../shell.ts";
  import { Button, Dot, Kbd } from "../../ui/index.ts";
  import NavItem from "./NavItem.svelte";

  const waiting = $derived(store.runs.filter((r) => r.state === "waiting_for_approval" || store.needsYou[r.id]).length);
  const running = $derived(runningCount(store.runs));
  const spend = $derived(spendToday(store.runs, store.now));
  const shortcut = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform || navigator.userAgent) ? "⌘N" : "Ctrl N";
  const teamsOpen = $derived(store.view === "agents" && agents.section === "teams");
  const agentsOpen = $derived(store.view === "agents" && agents.section === "agents");
</script>

<aside class="flex w-60 shrink-0 flex-col border-r border-line bg-panel-2">
  <div class="p-3">
    <Button variant="subtle" icon="plus" class="w-full justify-start" onclick={() => { store.continuing = null; store.showForm(); }}>
      <span class="flex-1 text-left">New run</span><Kbd>{shortcut}</Kbd>
    </Button>
  </div>

  <nav class="min-h-0 flex-1 space-y-4 overflow-y-auto px-3 pb-3" aria-label="Main">
    <div>
      <p class="px-2.5 pb-1.5 text-2xs font-semibold tracking-wider text-faint uppercase">Work</p>
      <NavItem icon="bell" active={store.view === "inbox"} count={inbox.count} attention onclick={() => store.showInbox()}>Inbox</NavItem>
      <NavItem icon="history" active={store.view === "runs"} onclick={() => store.showRuns()}>Runs</NavItem>
    </div>
    <div>
      <p class="px-2.5 pb-1.5 text-2xs font-semibold tracking-wider text-faint uppercase">Organisation</p>
      <NavItem icon="bot" active={agentsOpen} onclick={() => store.showAgents()}>Agents</NavItem>
      <NavItem icon="users" active={teamsOpen} onclick={() => store.showTeams()}>Teams</NavItem>
    </div>
    <div>
      <p class="px-2.5 pb-1.5 text-2xs font-semibold tracking-wider text-faint uppercase">Connect</p>
      <NavItem icon="key" active={store.view === "providers"} onclick={() => store.showProviders()}>Providers</NavItem>
      <NavItem icon="plug" active={store.view === "integrations"} count={integrations.attention} attention={integrations.attention > 0} onclick={() => store.showIntegrations()}>Integrations</NavItem>
      <NavItem icon="book" active={store.view === "memory"} onclick={() => store.showMemory()}>Memory</NavItem>
      <NavItem icon="folder" active={store.view === "documents"} onclick={() => store.showDocuments()}>Documents</NavItem>
    </div>
    <div>
      <p class="px-2.5 pb-1.5 text-2xs font-semibold tracking-wider text-faint uppercase">Operate</p>
      {#if import.meta.env.DEV}
        <NavItem icon="layers" active={store.view === "design"} onclick={() => store.showDesign()}>Design system</NavItem>
      {/if}
      <NavItem icon="sliders" active={store.view === "settings"} onclick={() => store.showSettings()}>Settings</NavItem>
    </div>
  </nav>

  <footer class="p-3">
    <div class="rounded-2xl border border-line bg-panel px-3.5 py-3 shadow-card">
      <p class="flex items-baseline justify-between text-sm text-muted"><span>Spend today</span><span class="mono tnum text-base font-semibold text-fg">{usd(spend)}</span></p>
      <p class="mt-2 flex items-center gap-1.5 text-xs text-muted">
        <Dot tone={running > 0 ? "ok" : "muted"} live={running > 0} />
        <span class="tnum">{running} running{waiting > 0 ? ` · ${waiting} waiting for you` : ""}</span>
      </p>
    </div>
  </footer>
</aside>
