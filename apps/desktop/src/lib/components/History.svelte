<script lang="ts">
  import { type Found, errorText, tryQuery } from "../api.ts";
  import { store } from "../stores/runs.svelte.ts";
  import { Badge, Button, Callout, Card, EmptyState, Input } from "../ui/index.ts";

  let query = $state("");
  let found = $state<Found | null>(null);
  let searching = $state(false);
  let error = $state<string | null>(null);

  async function search() {
    if (!query.trim()) return;
    searching = true;
    error = null;
    try {
      found = await tryQuery(query, "history");
    } catch (e) {
      error = errorText(e);
    } finally {
      searching = false;
    }
  }

  function open(id: string) {
    store.showRuns();
    void store.select(id);
  }

  const tone = (state: string | null) => (state === "completed" ? "ok" : state === "failed" ? "err" : "muted");
</script>

<div class="space-y-5">
  <Callout tone="info" icon="history">
    ZORIX keeps a short record of every run that ends: what you asked, which tools it used, who approved what, and how it ended. Runs
    can search it with <span class="mono">history.search</span>, and they see the most relevant records as past work.
  </Callout>
  {#if error}<Callout tone="err">{error}</Callout>{/if}
  <form class="flex gap-2" onsubmit={(e) => { e.preventDefault(); void search(); }}>
    <Input class="min-w-0 flex-1" bind:value={query} placeholder="Search past work, for example: staging deploy" aria-label="Search past work" />
    <Button type="submit" variant="primary" icon="search" loading={searching} disabled={!query.trim()}>Search</Button>
  </form>

  {#if found}
    {#if found.degraded}<p class="text-xs text-warn">{found.degraded}; showing keyword results.</p>{/if}
    {#each found.hits as hit}
      <Card interactive>
        <div class="flex items-start justify-between gap-3">
          <div class="min-w-0 flex-1">
            <div class="flex flex-wrap items-center gap-2">
              {#if hit.state}<Badge tone={tone(hit.state)} dot>{hit.state}</Badge>{/if}
              <span class="selectable text-md font-medium">{hit.title}</span>
            </div>
            <p class="selectable mt-1.5 text-sm text-muted">{hit.text}</p>
            <p class="mt-1.5 text-xs text-faint">
              {[hit.keyword_rank ? `keyword #${hit.keyword_rank}` : "", hit.vector_rank ? `vector #${hit.vector_rank}` : ""].filter(Boolean).join(" · ")}
            </p>
          </div>
          {#if hit.execution}<Button size="sm" icon="external" onclick={() => open(hit.execution!)}>Open run</Button>{/if}
        </div>
      </Card>
    {:else}
      <Card><EmptyState icon="history" title="Nothing found" text="No past work matches. Try other words." /></Card>
    {/each}
  {:else}
    <Card><EmptyState icon="history" title="Search what was done before" text="Type a few words from a past request." /></Card>
  {/if}
</div>
