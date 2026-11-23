<script lang="ts">
  import { t } from "../i18n/index.ts";
  import { decideRun, errorText } from "../api.ts";
  import { ago, shortId } from "../format.ts";
  import { inbox } from "../stores/inbox.svelte.ts";
  import { store } from "../stores/runs.svelte.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import { Badge, Button, EmptyState, Icon } from "../ui/index.ts";

  /** What runs proposed to remember and are waiting on you. A proposal is an approval of `memory.remember`; deciding it here is the same as on the run. */
  const items = $derived(inbox.approvals.filter((i) => i.kind === "approval" && i.tool === "memory.remember"));
  let busy = $state<string | null>(null);

  const text = (value: unknown) => (typeof value === "string" ? value : "");

  async function answer(item: (typeof items)[number], approve: boolean) {
    if (item.kind !== "approval") return;
    busy = item.id;
    try {
      await decideRun(item.execution, item.id, approve, item.approval.call_hash);
      toast.ok(approve ? t("memoryProposals.remembered") : t("memoryProposals.refused"));
      await store.refreshList();
      await store.refreshMemory(true);
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      busy = null;
    }
  }
</script>

{#if items.length === 0}
  <EmptyState icon="shield-check" title={t("memoryProposals.nothingToApprove")} text={t("memoryProposals.whenARunProposes")} />
{:else}
  <ul class="space-y-3" aria-label={t("memoryProposals.proposalsToApprove")}>
    {#each items as item (item.id)}
      {#if item.kind === "approval"}
        {@const args = (item.approval.arguments ?? {}) as Record<string, unknown>}
        <li class="overflow-hidden rounded-2xl border border-warn/40 bg-panel shadow-card">
          <div class="flex items-center gap-2 border-b border-line-soft bg-warn/8 px-4 py-2.5">
            <Icon name="book" size={15} class="text-warn" />
            <p class="min-w-0 flex-1 truncate text-base"><span class="font-semibold">{item.agent ?? t("memoryProposals.aRun")}</span> <span class="text-muted">{t("memoryProposals.proposesA", { value: text(args.kind) || "fact", value2: text(args.scope) || "global" })}</span></p>
            <Badge tone="warn">{t("memoryProposals.toApprove")}</Badge>
          </div>
          <div class="px-4 py-3">
            <p class="selectable text-md leading-relaxed">{text(args.content)}</p>
            <p class="mt-1.5 text-xs text-muted">{t("memoryProposals.fromRun")} <button type="button" class="mono underline decoration-line-strong underline-offset-2 hover:text-fg" onclick={() => void store.select(item.root)}>{shortId(item.root)}</button> {t("memoryProposals.waiting")} <span class="tnum">{ago(item.at, store.now).replace(" ago", "")}</span></p>
          </div>
          <div class="flex gap-2 border-t border-line-soft bg-panel-2 px-4 py-2.5">
            <Button size="sm" variant="primary" icon="check" loading={busy === item.id} onclick={() => answer(item, true)}>{t("common.approve")}</Button>
            <Button size="sm" icon="x" disabled={busy === item.id} onclick={() => answer(item, false)}>{t("common.refuse")}</Button>
          </div>
        </li>
      {/if}
    {/each}
  </ul>
{/if}
