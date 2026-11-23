<script lang="ts">
  import { fmt, t } from "../../i18n/index.ts";
  import { type AgentDetail, type AgentVersion, agentExport, agentRollback, agentVersions, errorText } from "../../api.ts";
  import { ago } from "../../format.ts";
  import { agents } from "../../stores/agents.svelte.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { Button, Callout, Card, DiffView, EmptyState, VersionTimeline } from "../../ui/index.ts";

  let { detail }: { detail: AgentDetail } = $props();
  const slug = $derived(detail.agent.slug);

  let versions = $state<AgentVersion[]>([]);
  let picked = $state<number | null>(null);
  let before = $state("");
  let after = $state("");
  let busy = $state<number | null>(null);
  let error = $state<string | null>(null);

  const items = $derived(
    versions.map((v) => ({
      version: v.version,
      hash: v.hash,
      note: v.note,
      at: `${fmt.stamp(v.created_at)} · ${ago(v.created_at)}`,
      current: v.version === detail.agent.current_version,
    })),
  );

  async function load() {
    try {
      versions = await agentVersions(slug);
      if (picked === null || !versions.some((v) => v.version === picked)) picked = versions[0]?.version ?? null;
      error = null;
    } catch (e) {
      error = errorText(e);
    }
  }
  $effect(() => {
    void detail.agent.current_version;
    void load();
  });

  // What changed in the picked version: the file as it was just before, against the file as it was then.
  $effect(() => {
    const at = picked;
    if (at === null) return;
    void (async () => {
      try {
        const index = versions.findIndex((v) => v.version === at);
        const older = versions[index + 1];
        const [now, was] = await Promise.all([agentExport(slug, at), older ? agentExport(slug, older.version) : Promise.resolve("")]);
        if (picked === at) [after, before] = [now, was];
      } catch (e) {
        error = errorText(e);
      }
    })();
  });

  async function rollback(version: number) {
    busy = version;
    try {
      const made = await agentRollback(slug, version);
      toast.ok(t("agents.agentVersions.isNowAtV", { slug, version: made.version, version2: version }));
      await agents.reload();
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      busy = null;
    }
  }
  const pickedVersion = $derived(versions.find((v) => v.version === picked));
  const older = $derived(versions[versions.findIndex((v) => v.version === picked) + 1]);
</script>

<div class="h-full overflow-y-auto">
  <div class="mx-auto grid max-w-5xl gap-6 p-8 lg:grid-cols-[22rem_minmax(0,1fr)]">
    {#if error}<div class="lg:col-span-2"><Callout tone="err" title={t("agents.agentVersions.versionsCouldNotBe")}>{error}</Callout></div>{/if}
    <section aria-labelledby="versions-title">
      <h2 id="versions-title" class="mb-3 text-lg font-semibold">{t("agents.agentVersions.history")}</h2>
      {#if items.length}
        <VersionTimeline {items} selected={picked} onselect={(v) => (picked = v)}>
          {#snippet actions(item)}
            {#if !item.current}<Button size="sm" icon="undo" loading={busy === item.version} onclick={() => rollback(item.version)} aria-label={t("agents.agentVersions.rollBackToVersion", { version: item.version })}>{t("agents.agentVersions.rollBack")}</Button>{/if}
          {/snippet}
        </VersionTimeline>
        <p class="mt-3 text-xs text-faint">{t("agents.agentVersions.aVersionIsNever")}</p>
      {:else}
        <EmptyState icon="history" title={t("agents.agentVersions.noVersions")} />
      {/if}
    </section>
    <section aria-labelledby="changes-title">
      <h2 id="changes-title" class="mb-3 text-lg font-semibold">
        {#if pickedVersion}{t("agents.agentVersions.whatChangedInV", { version: pickedVersion.version })}{:else}{t("agents.agentVersions.changes")}{/if}
      </h2>
      {#if pickedVersion}
        {#if older}
          <DiffView {before} {after} beforeLabel={`v${older.version}`} afterLabel={`v${pickedVersion.version}`} />
        {:else}
          <Callout tone="muted" title={t("agents.agentVersions.theFirstVersion")}>{t("agents.agentVersions.everythingInItIs")}</Callout>
        {/if}
      {/if}
    </section>
  </div>
</div>
