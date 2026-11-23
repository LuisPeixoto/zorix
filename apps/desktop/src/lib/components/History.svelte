<script lang="ts">
  import { t } from "../i18n/index.ts";
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
    {t("history.zorixKeepsAShort")} <span class="mono">history.search</span>{t("history.andTheySeeThe")}
  </Callout>
  {#if error}<Callout tone="err">{error}</Callout>{/if}
  <form class="flex gap-2" onsubmit={(e) => { e.preventDefault(); void search(); }}>
    <Input class="min-w-0 flex-1" bind:value={query} placeholder={t("history.searchPastWorkFor")} aria-label={t("history.searchPastWork")} />
    <Button type="submit" variant="primary" icon="search" loading={searching} disabled={!query.trim()}>{t("history.search")}</Button>
  </form>

  {#if found}
    {#if found.degraded}<p class="text-xs text-warn">{t("history.showingKeywordResults", { degraded: found.degraded })}</p>{/if}
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
              {[hit.keyword_rank ? t("history.keyword", { keyword_rank: hit.keyword_rank }) : "", hit.vector_rank ? t("history.vector", { vector_rank: hit.vector_rank }) : ""].filter(Boolean).join(" · ")}
            </p>
          </div>
          {#if hit.execution}<Button size="sm" icon="external" onclick={() => open(hit.execution!)}>{t("history.openRun")}</Button>{/if}
        </div>
      </Card>
    {:else}
      <EmptyState icon="history" title={t("history.nothingFound")} text={t("history.noPastWorkMatches")} />
    {/each}
  {:else}
    <EmptyState icon="history" title={t("history.searchWhatWasDone")} text={t("history.typeAFewWords")} />
  {/if}
</div>
