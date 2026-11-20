<script lang="ts">
  import { type TreeApproval, decideRun, errorText } from "../../api.ts";
  import { json } from "../../format.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { Badge, Button, CodeBlock, Icon } from "../../ui/index.ts";

  /** Approvals that wait below a run: a sub-run asked, and the person decides here, on the run they started. */
  let { items, onanswered, compact = false }: { items: TreeApproval[]; onanswered?: () => void; compact?: boolean } = $props();
  let busy = $state<string | null>(null);

  async function answer(item: TreeApproval, approve: boolean) {
    busy = item.approval.id;
    try {
      await decideRun(item.execution, item.approval.id, approve, item.approval.call_hash);
      onanswered?.();
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      busy = null;
    }
  }
</script>

{#each items as item (item.approval.id)}
  <section class="overflow-hidden rounded-2xl border border-warn/50 bg-panel" aria-label="Approval asked by {item.agent ?? 'a sub-run'}">
    <header class="flex items-center gap-2.5 border-b border-line-soft bg-warn/8 px-4 py-2.5">
      <Icon name="shield" size={16} class="text-warn" />
      <p class="min-w-0 flex-1 text-md font-semibold"><span class="mono">{item.approval.tool}</span> <span class="font-normal text-muted">needs your approval</span></p>
      <Badge tone="warn">{item.agent ? `asked by ${item.agent}` : "sub-run"}{item.depth > 1 ? ` · level ${item.depth}` : ""}</Badge>
    </header>
    <div class="space-y-3 p-4">
      <p class="text-sm text-muted">{item.approval.reason}</p>
      {#if !compact}<CodeBlock code={json(item.approval.arguments)} language="json" maxHeight={180} label="Arguments" />{/if}
      <div class="flex gap-2">
        <Button variant="primary" icon="check" loading={busy === item.approval.id} onclick={() => answer(item, true)}>Approve</Button>
        <Button icon="x" disabled={busy === item.approval.id} onclick={() => answer(item, false)}>Refuse</Button>
      </div>
    </div>
  </section>
{/each}
