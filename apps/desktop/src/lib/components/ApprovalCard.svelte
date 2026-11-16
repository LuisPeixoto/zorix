<script lang="ts">
  import type { Approval } from "../api.ts";
  import { json } from "../format.ts";
  import { store } from "../stores/runs.svelte.ts";
  import { Button, Callout, Field, Input } from "../ui/index.ts";

  let { approval }: { approval: Approval } = $props();
  let note = $state("");
</script>

<Callout tone="warn" icon="shield" title="Approval needed">
  <p>
    The run wants to use <code class="mono text-fg">{approval.tool}</code>. Nothing has happened yet.
  </p>
  <p class="mt-1">Why you are asked: {approval.reason}</p>
  <pre class="selectable mono mt-3 max-h-48 overflow-auto rounded-lg border border-line bg-panel p-3 text-sm text-fg">{json(approval.arguments)}</pre>
  <p class="mt-1.5 text-xs text-faint">
    Your answer applies to exactly these arguments (fingerprint <span class="mono">{approval.call_hash.slice(0, 12)}…</span>).
  </p>
  <div class="mt-3">
    <Field label="Note (optional; the model reads it if you refuse)">
      {#snippet children(f)}<Input id={f.id} bind:value={note} maxlength={300} />{/snippet}
    </Field>
  </div>
  {#snippet actions()}
    <Button variant="primary" icon="check" disabled={store.busy} onclick={() => store.decide(approval.id, approval.call_hash, true, note)}>Approve</Button>
    <Button icon="x" disabled={store.busy} onclick={() => store.decide(approval.id, approval.call_hash, false, note)}>Refuse</Button>
  {/snippet}
</Callout>
