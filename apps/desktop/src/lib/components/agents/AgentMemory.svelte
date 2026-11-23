<script lang="ts">
  import { t } from "../../i18n/index.ts";
  import { type AgentDetail, type MemoryRow, addMemory, errorText, forgetMemory, listMemory } from "../../api.ts";
  import { ago } from "../../format.ts";
  import { store } from "../../stores/runs.svelte.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { Badge, Button, Callout, Card, EmptyState, Field, IconButton, Input, Segmented } from "../../ui/index.ts";

  let { detail }: { detail: AgentDetail } = $props();
  const scope = $derived(`agent:${detail.agent.id}`);

  let rows = $state<MemoryRow[]>([]);
  let content = $state("");
  let kind = $state<"fact" | "preference">("fact");
  let saving = $state(false);
  let error = $state<string | null>(null);
  const own = $derived(rows.filter((r) => r.scope === scope && r.status === "active"));

  async function load() {
    try {
      rows = await listMemory(false);
      error = null;
    } catch (e) {
      error = errorText(e);
    }
  }
  $effect(() => {
    void scope;
    void load();
  });

  async function add() {
    saving = true;
    try {
      toast.ok(await addMemory({ content: content.trim(), kind, scope }));
      content = "";
      await load();
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      saving = false;
    }
  }

  async function forget(id: string) {
    try {
      await forgetMemory(id);
      toast.ok(t("agents.agentMemory.forgotten"));
      await load();
    } catch (e) {
      toast.err(errorText(e));
    }
  }
</script>

<div class="h-full overflow-y-auto">
  <div class="mx-auto flex max-w-3xl flex-col gap-5 p-8">
    {#if error}<Callout tone="err" title={t("agents.agentMemory.memoryCouldNotBe")}>{error}</Callout>{/if}
    {#if !detail.version.spec.agent_memory}
      <Callout tone="warn" title={t("agents.agentMemory.thisAgentHasNo")}>{t("agents.agentMemory.itDoesNotSee")}</Callout>
    {/if}
    <Callout tone="muted" title={t("agents.agentMemory.onlySeesThese", { name: detail.version.spec.name })} icon="lock">
      {t("agents.agentMemory.factsHereAreVisible")}
    </Callout>

    <Card title={t("agents.agentMemory.addAFact")} icon="plus">
      <form class="space-y-3" onsubmit={(e) => { e.preventDefault(); void add(); }}>
        <Field label={t("agents.agentMemory.whatShouldItRemember")}>{#snippet children(f)}<Input id={f.id} bind:value={content} maxlength={2000} placeholder={t("agents.agentMemory.alwaysGiveAmountsWith")} />{/snippet}</Field>
        <div class="flex items-center gap-3">
          <Segmented label={t("common.kind")} bind:value={kind} options={[{ value: "fact", label: "Fact" }, { value: "preference", label: "Preference" }]} />
          <span class="flex-1"></span>
          <Button type="submit" variant="primary" icon="check" loading={saving} disabled={!content.trim()}>{t("agents.agentMemory.remember")}</Button>
        </div>
      </form>
    </Card>

    <section aria-labelledby="own-title">
      <h2 id="own-title" class="mb-3 text-lg font-semibold">{t("agents.agentMemory.whatItRemembers")} <span class="text-sm font-normal text-faint">{own.length}</span></h2>
      {#if own.length === 0}
        <EmptyState icon="book" title={t("agents.agentMemory.nothingYet")} text={t("agents.agentMemory.factsThatOnlyThis")} />
      {:else}
        <ul class="space-y-2">
          {#each own as m (m.id)}
            <li class="raised flex items-start gap-3 rounded-xl border border-line bg-panel px-3.5 py-3">
              <div class="min-w-0 flex-1">
                <p class="selectable text-base">{m.content}</p>
                <p class="mt-1 flex items-center gap-2 text-xs text-faint"><Badge tone="muted">{m.kind}</Badge> <span>{t("agents.agentMemory.addedBy", { by: m.by })}</span> <span aria-hidden="true">·</span> <time>{ago(m.created_at, store.now)}</time> {#if m.use_count}<span aria-hidden="true">·</span> <span>used {m.use_count}×</span>{/if}</p>
              </div>
              <IconButton icon="trash" label={t("agents.agentMemory.forgetThis")} onclick={() => forget(m.id)} />
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  </div>
</div>
