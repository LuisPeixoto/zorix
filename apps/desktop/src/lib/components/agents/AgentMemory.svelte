<script lang="ts">
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
      toast.ok("Forgotten.");
      await load();
    } catch (e) {
      toast.err(errorText(e));
    }
  }
</script>

<div class="h-full overflow-y-auto">
  <div class="mx-auto flex max-w-3xl flex-col gap-5 p-8">
    {#if error}<Callout tone="err" title="Memory could not be read">{error}</Callout>{/if}
    {#if !detail.version.spec.agent_memory}
      <Callout tone="warn" title="This agent has no memory of its own">It does not see these facts. Turn on "Its own memory" on the Definition tab.</Callout>
    {/if}
    <Callout tone="muted" title="Only {detail.version.spec.name} sees these" icon="lock">
      Facts here are visible to this agent and to nobody else. When it wants to remember something, it asks in the chat and you decide; nothing is kept without your approval.
    </Callout>

    <Card title="Add a fact" icon="plus">
      <form class="space-y-3" onsubmit={(e) => { e.preventDefault(); void add(); }}>
        <Field label="What should it remember?">{#snippet children(f)}<Input id={f.id} bind:value={content} maxlength={2000} placeholder="Always give amounts with their currency." />{/snippet}</Field>
        <div class="flex items-center gap-3">
          <Segmented label="Kind" bind:value={kind} options={[{ value: "fact", label: "Fact" }, { value: "preference", label: "Preference" }]} />
          <span class="flex-1"></span>
          <Button type="submit" variant="primary" icon="check" loading={saving} disabled={!content.trim()}>Remember</Button>
        </div>
      </form>
    </Card>

    <section aria-labelledby="own-title">
      <h2 id="own-title" class="mb-3 text-lg font-semibold">What it remembers <span class="text-sm font-normal text-faint">{own.length}</span></h2>
      {#if own.length === 0}
        <Card padding="none"><EmptyState icon="book" title="Nothing yet" text="Facts that only this agent should know will appear here." /></Card>
      {:else}
        <ul class="space-y-2">
          {#each own as m (m.id)}
            <li class="raised flex items-start gap-3 rounded-xl border border-line bg-panel px-3.5 py-3">
              <div class="min-w-0 flex-1">
                <p class="selectable text-base">{m.content}</p>
                <p class="mt-1 flex items-center gap-2 text-xs text-faint"><Badge tone="muted">{m.kind}</Badge> <span>added by {m.by}</span> <span aria-hidden="true">·</span> <time>{ago(m.created_at, store.now)}</time> {#if m.use_count}<span aria-hidden="true">·</span> <span>used {m.use_count}×</span>{/if}</p>
              </div>
              <IconButton icon="trash" label="Forget this" onclick={() => forget(m.id)} />
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  </div>
</div>
