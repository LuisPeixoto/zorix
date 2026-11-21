<script lang="ts">
  import { type CatalogEntry, type ConnectionKind } from "../../api.ts";
  import { ago } from "../../format.ts";
  import { integrations } from "../../stores/integrations.svelte.ts";
  import { store } from "../../stores/runs.svelte.ts";
  import {
    Badge, BrandLogo, Button, Callout, Card, Chip, EmptyState, Icon, IconButton, Input, List, ListRow, PageHeader, Segmented, Tabs,
  } from "../../ui/index.ts";
  import AddServerModal from "./AddServerModal.svelte";
  import ConnectModal from "./ConnectModal.svelte";
  import { KINDS } from "./kinds.ts";
  import ImportModal from "./ImportModal.svelte";
  import ServerCard from "./ServerCard.svelte";
  import ServerDrawer from "./ServerDrawer.svelte";

  let tab = $state(store.integrationsTab);
  $effect(() => {
    tab = store.integrationsTab;
  });
  $effect(() => {
    store.integrationsTab = tab;
  });

  let query = $state("");
  let show = $state("all");
  const wants = (s: (typeof integrations.servers)[number]) => s.enabled && (s.tools_to_review > 0 || s.state === "needs_login" || s.state === "failed");
  const attention = $derived(integrations.servers.filter(wants));
  const failed = $derived(integrations.servers.filter((s) => s.enabled && s.state === "failed"));
  const listed = $derived(show === "attention" ? attention : integrations.servers);
  let category = $state("All");
  let addOpen = $state(false);
  let addEntry = $state<CatalogEntry | null>(null);
  let connectOpen = $state(false);
  let connectKind = $state<ConnectionKind | null>(null);
  let importOpen = $state(false);

  const tabs = $derived([
    { value: "connected", label: "Connected", count: integrations.servers.length },
    { value: "catalogue", label: "Catalogue", count: integrations.catalog.length },
    { value: "connections", label: "Connections", count: integrations.connections.length },
  ]);
  const categories = $derived(["All", ...new Set(integrations.catalog.map((e) => e.category))]);
  const shown = $derived(
    integrations.catalog.filter(
      (e) =>
        (category === "All" || e.category === category) &&
        (!query.trim() || `${e.name} ${e.summary} ${e.category}`.toLowerCase().includes(query.trim().toLowerCase())),
    ),
  );
  const added = (entry: CatalogEntry) => integrations.servers.some((s) => s.logo === entry.logo);

  function addFrom(entry: CatalogEntry | null) {
    addEntry = entry;
    addOpen = true;
  }
  function connect(kind: ConnectionKind) {
    connectKind = kind;
    connectOpen = true;
  }
  const kindKeys = Object.keys(KINDS) as ConnectionKind[];
  const python = $derived(integrations.sandbox?.python ?? false);
  const pct = $derived(integrations.installing ? Math.min(100, Math.round((integrations.installing.done / Math.max(1, integrations.installing.total)) * 100)) : 0);
</script>

<div class="flex h-full flex-col overflow-y-auto">
  <div class="mx-auto w-full max-w-5xl shrink-0 p-8">
    <PageHeader title="Integrations" subtitle="Connect ZORIX to tools, services and data. A run can only reach what you grant it, and anything that changes something asks you first." glow>
      {#snippet actions()}
        <Button icon="download" onclick={() => (importOpen = true)}>Import</Button>
        <Button variant="primary" icon="plus" onclick={() => addFrom(null)}>Add a server</Button>
      {/snippet}
    </PageHeader>

    {#if integrations.error}<Callout tone="err" class="mb-4">{integrations.error}</Callout>{/if}
    <Tabs {tabs} bind:value={tab} label="Integrations" class="mb-5" />

    {#if tab === "connected"}
      {#if !integrations.loaded}
        <div class="grid gap-4 sm:grid-cols-2" aria-busy="true" aria-label="Loading"><div class="skeleton h-28"></div><div class="skeleton h-28"></div></div>
      {:else if integrations.servers.length === 0}
        <Card padding="none">
          <EmptyState icon="plug" title="No servers yet" text="MCP servers give a run new tools: your repositories, tickets, documents, a browser. Pick one from the catalogue, bring in the ones you already use, or add your own.">
            {#snippet actions()}
              <Button variant="primary" icon="grid" onclick={() => (tab = "catalogue")}>Browse the catalogue</Button>
              <Button icon="download" onclick={() => (importOpen = true)}>Import from another app</Button>
            {/snippet}
          </EmptyState>
        </Card>
      {:else}
        {#each failed as server (server.name)}
          <Callout tone="err" title="{server.name} failed" class="mb-4">
            {#snippet actions()}<Button size="sm" onclick={() => integrations.open(server.name)}>Open {server.name}</Button>{/snippet}
            <span class="selectable">{server.reason ?? "The server stopped answering."}</span> Fix it, or switch the server off so runs do not wait for it.
          </Callout>
        {/each}
        <div class="mb-4">
          <Segmented
            label="Show"
            bind:value={show}
            options={[
              { value: "all", label: `All ${integrations.servers.length}` },
              { value: "attention", label: `Needs attention ${attention.length}` },
            ]}
          />
        </div>
        {#if listed.length === 0}
          <p class="rounded-2xl border border-dashed border-line-strong bg-panel px-6 py-8 text-center text-base text-muted">Nothing needs attention.</p>
        {:else}
          <div class="grid gap-4 md:grid-cols-2">
            {#each listed as server (server.name)}
              <ServerCard {server} onopen={() => integrations.open(server.name)} />
            {/each}
          </div>
        {/if}
      {/if}
    {:else if tab === "catalogue"}
      <div class="mb-4 flex flex-wrap items-center gap-3">
        <Input class="w-72" icon="search" bind:value={query} placeholder="Search the catalogue" aria-label="Search the catalogue" />
        <div class="flex flex-wrap gap-1.5" role="group" aria-label="Category">
          {#each categories as name (name)}<Chip selected={category === name} onclick={() => (category = name)}>{name}</Chip>{/each}
        </div>
      </div>
      <div class="grid gap-4 md:grid-cols-2">
        {#each shown as entry (entry.id)}
          <div class="shadow-card flex flex-col gap-3 rounded-2xl border border-line bg-panel p-4">
            <div class="flex items-start gap-3">
              <BrandLogo name={entry.logo} size={40} />
              <div class="min-w-0 flex-1">
                <p class="text-lg font-semibold">{entry.name}</p>
                <p class="mt-0.5 text-sm text-muted">{entry.summary}</p>
              </div>
            </div>
            <div class="mt-auto flex items-center gap-2">
              <Badge tone="muted">{entry.category}</Badge>
              <Badge tone="muted">{entry.transport.kind === "stdio" ? "runs here" : "remote"}</Badge>
              <span class="ml-auto">
                {#if added(entry)}
                  <Badge tone="ok" dot>Added</Badge>
                {:else}
                  <Button size="sm" icon="plus" aria-label="Add {entry.name}" onclick={() => addFrom(entry)}>Add</Button>
                {/if}
              </span>
            </div>
          </div>
        {:else}
          <p class="col-span-full py-10 text-center text-base text-muted">Nothing in the catalogue matches. <button type="button" class="text-accent underline" onclick={() => addFrom(null)}>Add your own server</button>.</p>
        {/each}
      </div>
      <p class="mt-4 text-xs text-muted">The catalogue is a short reviewed list, checked {integrations.catalog.length ? "against the real servers" : ""}. Anything not here can be added with its command or address.</p>
    {:else}
      <div class="grid gap-3 sm:grid-cols-3">
        {#each kindKeys as kind (kind)}
          <button
            type="button"
            onclick={() => connect(kind)}
            class="shadow-card group flex items-center gap-3 rounded-2xl border border-line bg-panel p-3 text-left transition duration-150 hover:-translate-y-px hover:border-line-strong hover:shadow-3"
          >
            <BrandLogo name={KINDS[kind].logo} size={36} />
            <span class="min-w-0 flex-1"><span class="block text-md font-medium">{KINDS[kind].title}</span><span class="block truncate text-xs text-muted">{KINDS[kind].summary}</span></span>
            <Icon name="plus" size={15} class="text-faint group-hover:text-accent" />
          </button>
        {/each}
      </div>

      <h2 class="mt-8 mb-2 text-lg font-semibold">Your connections</h2>
      {#if integrations.connections.length === 0}
        <Card padding="none"><p class="px-4 py-8 text-center text-base text-muted">None yet. Pick a service above.</p></Card>
      {:else}
        <Card padding="none">
          <List class="rounded-none border-0 shadow-none" label="Connections">
            {#each integrations.connections as c (c.id)}
              {@const spec = KINDS[c.kind]}
              <ListRow>
                <BrandLogo name={spec.logo} size={34} />
                <span class="min-w-0 flex-1">
                  <span class="block text-md font-medium">{c.name} <span class="ml-1 text-xs font-normal text-muted">{spec.title}</span></span>
                  <span class="block truncate text-xs text-muted">
                    {#if c.last_test}{c.last_test.ok ? "Worked" : "Failed"} {ago(c.last_test.at, store.now)} · {c.last_test.note}{:else}Not tested yet{/if}
                  </span>
                </span>
                {#if c.last_test}<Badge tone={c.last_test.ok ? "ok" : "err"} dot>{c.last_test.ok ? "ok" : "failed"}</Badge>{/if}
                {#if c.has_secret}<Badge tone="muted"><Icon name="lock" size={11} /> secret</Badge>{/if}
                <Button size="sm" aria-label="Test {c.name}" onclick={() => integrations.testConnection(c.name)}>Test</Button>
                <IconButton label="Remove {c.name}" icon="trash" variant="danger" size="sm" onclick={() => integrations.removeConnection(c.name)} />
              </ListRow>
            {/each}
          </List>
        </Card>
      {/if}

      <h2 class="mt-8 mb-2 text-lg font-semibold">Code sandbox</h2>
      <Card>
        <div class="flex flex-wrap items-center gap-4">
          <span class="grid size-10 place-items-center rounded-xl bg-accent-soft text-accent"><Icon name="code" size={20} /></span>
          <div class="min-w-0 flex-1">
            <p class="text-md font-medium">Run short programs safely</p>
            <p class="text-sm text-muted">JavaScript and Python run in a WebAssembly sandbox with no network, no access to your files unless you allow it, and hard limits on memory, time and output.</p>
          </div>
        </div>
        <div class="mt-4 flex flex-wrap items-center gap-3">
          <Badge tone="ok" dot>JavaScript ready</Badge>
          {#if python}
            <Badge tone="ok" dot>Python ready</Badge>
          {:else if integrations.installing}
            <div class="flex min-w-48 flex-1 items-center gap-3" role="progressbar" aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100} aria-label="Downloading Python">
              <div class="h-1.5 flex-1 overflow-hidden rounded-full bg-hover"><div class="h-full rounded-full bg-accent transition-[width] duration-200" style:width="{pct}%"></div></div>
              <span class="tnum text-xs text-muted">{pct}%</span>
            </div>
          {:else}
            <Button size="sm" icon="download" onclick={() => integrations.installPython()}>Install Python (about {integrations.sandbox?.python_megabytes ?? 26} MB)</Button>
          {/if}
        </div>
        <p class="mt-3 text-xs text-muted">Python is downloaded once and checked against a fixed checksum. A run needs <span class="mono">code:python</span> or <span class="mono">code:javascript</span> to use them.</p>
      </Card>
    {/if}
  </div>
</div>

{#if integrations.openName}<ServerDrawer />{/if}
<AddServerModal bind:open={addOpen} entry={addEntry} />
<ConnectModal bind:open={connectOpen} kind={connectKind} />
<ImportModal bind:open={importOpen} />
