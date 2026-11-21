<script lang="ts">
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
      at: `${new Date(v.created_at).toLocaleString()} · ${ago(v.created_at)}`,
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
      toast.ok(`${slug} is now at v${made.version}, a copy of v${version}.`);
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
    {#if error}<div class="lg:col-span-2"><Callout tone="err" title="Versions could not be read">{error}</Callout></div>{/if}
    <section aria-labelledby="versions-title">
      <h2 id="versions-title" class="mb-3 text-lg font-semibold">History</h2>
      {#if items.length}
        <VersionTimeline {items} selected={picked} onselect={(v) => (picked = v)}>
          {#snippet actions(item)}
            {#if !item.current}<Button size="sm" icon="undo" loading={busy === item.version} onclick={() => rollback(item.version)} aria-label={`Roll back to version ${item.version}`}>Roll back</Button>{/if}
          {/snippet}
        </VersionTimeline>
        <p class="mt-3 text-xs text-faint">A version is never edited. Rolling back writes a new version that copies an old one, so nothing is lost.</p>
      {:else}
        <EmptyState icon="history" title="No versions" />
      {/if}
    </section>
    <section aria-labelledby="changes-title">
      <h2 id="changes-title" class="mb-3 text-lg font-semibold">
        {#if pickedVersion}What changed in v{pickedVersion.version}{:else}Changes{/if}
      </h2>
      {#if pickedVersion}
        {#if older}
          <DiffView {before} {after} beforeLabel={`v${older.version}`} afterLabel={`v${pickedVersion.version}`} />
        {:else}
          <Callout tone="muted" title="The first version">Everything in it is new. Pick a later version to see what changed.</Callout>
        {/if}
      {/if}
    </section>
  </div>
</div>
