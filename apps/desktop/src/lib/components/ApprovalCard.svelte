<script lang="ts">
  import type { Approval } from "../api.ts";
  import { json } from "../format.ts";
  import { store } from "../stores/runs.svelte.ts";
  import { Badge, Button, Callout, Field, Input } from "../ui/index.ts";

  let { approval }: { approval: Approval } = $props();
  let note = $state("");

  // A proposal to remember something gets a layout of its own: what would be remembered, where, and what it would replace.
  const remember = $derived(approval.tool === "memory.remember");
  const args = $derived((approval.arguments ?? {}) as Record<string, unknown>);
  const replaces = $derived(Array.isArray(args.replaces) ? (args.replaces as string[]) : []);
  const scopeText = $derived(typeof args.scope === "string" ? args.scope : "global");
  const subject = $derived(typeof args.subject === "string" ? args.subject : "");
  const affected = $derived(
    store.memory.filter(
      (m) => m.status === "active" && (replaces.some((r) => r.replace(/^m:/, "") === m.id.replace(/^m:/, "")) || (subject && m.subject === subject && m.scope === scopeText)),
    ),
  );
  $effect(() => {
    if (remember) void store.refreshMemory(false);
  });
</script>

{#if remember}
  <Callout tone="warn" icon="book" title="A run wants to remember something">
    <p>If you approve, this is saved and shown to future runs. Nothing is saved until you do.</p>
    <blockquote class="selectable mt-3 rounded-lg border border-line bg-panel p-3 text-md leading-relaxed text-fg">{String(args.content ?? "")}</blockquote>
    <div class="mt-2.5 flex flex-wrap gap-1.5">
      <Badge tone="muted">{String(args.kind ?? "fact")}</Badge>
      <Badge tone="accent">{scopeText === "global" ? "Everywhere" : scopeText}</Badge>
      {#if subject}<Badge tone="muted">about: {subject}</Badge>{/if}
      {#if args.valid_until}<Badge tone="muted">until {String(args.valid_until)}</Badge>{/if}
    </div>
    {#if affected.length}
      <div class="mt-3 rounded-lg border border-warn/40 bg-warn/8 p-3">
        <p class="text-sm font-medium">This would replace:</p>
        {#each affected as old (old.id)}<p class="selectable mt-1 text-sm">{old.content}</p>{/each}
      </div>
    {/if}
    <p class="mt-3 text-xs text-faint">
      You cannot edit a proposal before approving it. Refuse it with a note, or approve it and edit it under Memory.
      (Fingerprint <span class="mono">{approval.call_hash.slice(0, 12)}…</span>)
    </p>
    <div class="mt-3">
      <Field label="Note (optional; the model reads it if you refuse)">
        {#snippet children(f)}<Input id={f.id} bind:value={note} maxlength={300} />{/snippet}
      </Field>
    </div>
    {#snippet actions()}
      <Button variant="primary" icon="check" disabled={store.busy} onclick={() => store.decide(approval.id, approval.call_hash, true, note)}>Remember it</Button>
      <Button icon="x" disabled={store.busy} onclick={() => store.decide(approval.id, approval.call_hash, false, note)}>Refuse</Button>
    {/snippet}
  </Callout>
{:else}

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
{/if}
