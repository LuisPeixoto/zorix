<script lang="ts">
  import type { Approval } from "../api.ts";
  import { json } from "../format.ts";
  import { store } from "../stores/runs.svelte.ts";

  let { approval }: { approval: Approval } = $props();
  let note = $state("");
</script>

<section class="rounded-lg border border-warn/60 bg-warn/10 p-4" aria-label="Approval needed">
  <h3 class="text-base font-semibold text-warn">Approval needed</h3>
  <p class="mt-1 text-muted">
    The run wants to use <code class="font-mono text-fg">{approval.tool}</code>. Nothing has happened yet.
  </p>
  <p class="mt-1 text-muted">Why you are asked: {approval.reason}</p>
  <pre class="mt-3 max-h-48 overflow-auto rounded-md border border-line bg-panel p-3 font-mono text-xs">{json(approval.arguments)}</pre>
  <p class="mt-1 text-xs text-faint">
    Your answer applies to exactly these arguments (fingerprint {approval.call_hash.slice(0, 12)}…).
  </p>
  <label class="mt-3 block text-xs text-muted" for="note">Note (optional, the model reads it if you refuse)</label>
  <input
    id="note"
    class="mt-1 w-full rounded-md border border-line bg-panel px-3 py-1.5 text-fg"
    bind:value={note}
    maxlength="300"
  />
  <div class="mt-3 flex gap-2">
    <button
      class="rounded-md bg-accent px-4 py-1.5 font-medium text-accent-fg"
      disabled={store.busy}
      onclick={() => store.decide(approval.id, approval.call_hash, true, note)}>Approve</button
    >
    <button
      class="rounded-md border border-line bg-panel px-4 py-1.5 font-medium text-fg hover:bg-hover"
      disabled={store.busy}
      onclick={() => store.decide(approval.id, approval.call_hash, false, note)}>Refuse</button
    >
  </div>
</section>
