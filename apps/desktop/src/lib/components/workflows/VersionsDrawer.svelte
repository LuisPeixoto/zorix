<script lang="ts">
  import { t } from "../../i18n/index.ts";
  import { type WfDetail, errorText, workflowOutline, workflowRollback } from "../../api.ts";
  import { ago } from "../../format.ts";
  import { store } from "../../stores/runs.svelte.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { Badge, Button, Callout, Checkbox, DiffView, Drawer, VersionTimeline } from "../../ui/index.ts";

  let { open = $bindable(false), id, detail, onchanged }: { open?: boolean; id: string; detail: WfDetail; onchanged: () => void } = $props();

  let picked = $state<number | null>(null);
  let before = $state("");
  let after = $state("");
  let draftBefore = $state("");
  let draftAfter = $state("");
  let asking = $state<number | null>(null);
  let confirmed = $state(false);
  let busy = $state(false);

  const live = $derived(detail.workflow.live_version);
  const items = $derived(detail.versions.map((v) => ({ version: v.version, hash: v.hash, note: v.note, at: ago(v.created_at, store.now), current: v.version === live })));
  const selected = $derived(detail.versions.find((v) => v.version === picked) ?? null);

  $effect(() => {
    if (!open) return;
    if (picked === null || !detail.versions.some((v) => v.version === picked)) picked = detail.versions[0]?.version ?? null;
  });

  // What the selected version changed, against the one before it.
  $effect(() => {
    if (!open || picked === null) return;
    const number = picked;
    const index = detail.versions.findIndex((v) => v.version === number);
    const previous = detail.versions[index + 1];
    void (async () => {
      try {
        after = await workflowOutline(id, String(number));
        before = previous ? await workflowOutline(id, String(previous.version)) : "";
      } catch (e) {
        toast.err(errorText(e));
      }
    })();
  });

  // What is in the draft and not published yet.
  $effect(() => {
    if (!open || !detail.unpublished_changes || live === null) return;
    void (async () => {
      draftBefore = await workflowOutline(id, String(live)).catch(() => "");
      draftAfter = await workflowOutline(id, "draft").catch(() => "");
    })();
  });

  async function rollBack(version: number, confirm: boolean) {
    busy = true;
    try {
      const made = await workflowRollback(id, version, confirm);
      toast.ok(t("workflows.versionsDrawer.publishedVACopy", { version: made.version, version2: version }));
      asking = null;
      confirmed = false;
      onchanged();
    } catch (e) {
      const text = errorText(e);
      if (/confirm/i.test(text) && !confirm) asking = version;
      else toast.err(text);
    } finally {
      busy = false;
    }
  }
</script>

<Drawer bind:open title={t("common.versions")} subtitle={t("workflows.versionsDrawer.everyPublishIsA")} icon="history" width="lg">
  {#if detail.unpublished_changes && live !== null}
    <section class="mb-5">
      <h3 class="mb-2 flex items-center gap-2 text-sm font-semibold">{t("workflows.versionsDrawer.unpublishedChanges")} <Badge tone="muted">draft</Badge></h3>
      <DiffView before={draftBefore} after={draftAfter} beforeLabel="v{live} live" afterLabel="Draft" />
    </section>
  {/if}
  {#if items.length === 0}
    <Callout tone="info" title={t("workflows.versionsDrawer.nothingIsPublishedYet")}>{t("workflows.versionsDrawer.publishTheDraftTo")}</Callout>
  {:else}
    <VersionTimeline {items} selected={picked} onselect={(v) => (picked = v)}>
      {#snippet actions(item)}
        {#if !item.current}<Button size="sm" disabled={busy} onclick={() => void rollBack(item.version, false)}>{t("workflows.versionsDrawer.rollBackToV", { version: item.version })}</Button>{/if}
      {/snippet}
    </VersionTimeline>
    {#if selected}
      <section class="mt-5">
        <h3 class="mb-2 text-sm font-semibold">{t("workflows.versionsDrawer.whatV")}{selected.version} changed{selected.confirmed ? t("workflows.versionsDrawer.unattendedStepsConfirmed", { count: selected.confirmed }) : ""}</h3>
        <DiffView {before} {after} beforeLabel={detail.versions.some((v) => v.version === selected.version - 1) ? `v${selected.version - 1}` : "Nothing"} afterLabel="v{selected.version}" />
      </section>
    {/if}
  {/if}
  {#if asking !== null}
    <div class="mt-5">
      <Callout tone="warn" title={t("workflows.versionsDrawer.thisVersionSendsWithout")}>
        {t("workflows.versionsDrawer.someOfItsSteps")}
        <div class="mt-2.5"><Checkbox bind:checked={confirmed}>{t("workflows.versionsDrawer.iConfirmThoseSteps")}</Checkbox></div>
        {#snippet actions()}<Button variant="primary" size="sm" disabled={!confirmed || busy} onclick={() => asking !== null && void rollBack(asking, true)}>{t("workflows.versionsDrawer.rollBackToV2", { asking })}</Button>{/snippet}
      </Callout>
    </div>
  {/if}
</Drawer>
