<script lang="ts">
  import { onMount } from "svelte";

  import History from "./History.svelte";
  import MemoryProposals from "./MemoryProposals.svelte";

  import { type MemoryRow, errorText, memoryUsage } from "../api.ts";
  import { ago } from "../format.ts";
  import { agents } from "../stores/agents.svelte.ts";
  import { inbox } from "../stores/inbox.svelte.ts";
  import { store } from "../stores/runs.svelte.ts";
  import { Badge, Button, Callout, Card, EmptyState, Field, Input, Modal, PageHeader, Segmented, Select, Tabs, Textarea } from "../ui/index.ts";

  let tab = $state(store.startTab);
  let scope = $state("all");
  let query = $state("");
  let showOld = $state(false);

  const rows = $derived(
    store.memory.filter(
      (m) =>
        (scope === "all" || m.scope === scope || (scope === "project" && m.scope.startsWith("project:")) || (scope === "conversation" && m.scope.startsWith("conversation:")) || (scope === "agent" && m.scope.startsWith("agent:"))) &&
        (showOld || m.status === "active") &&
        (!query.trim() || `${m.content} ${m.subject ?? ""}`.toLowerCase().includes(query.trim().toLowerCase())),
    ),
  );
  const proposals = $derived(inbox.approvals.filter((i) => i.kind === "approval" && i.tool === "memory.remember").length);
  const activeCount = $derived(store.memory.filter((m) => m.status === "active").length);

  onMount(() => void store.refreshMemory(true));

  // Add and edit share one dialog.
  let editing = $state<MemoryRow | null>(null);
  let dialog = $state(false);
  let content = $state("");
  let kind = $state<"fact" | "preference">("fact");
  let where = $state("global");
  let subject = $state("");
  let until = $state("");
  let formError = $state<string | null>(null);
  let projectPath = $state("");

  function openAdd() {
    editing = null;
    content = "";
    kind = "fact";
    where = "global";
    subject = "";
    until = "";
    projectPath = "";
    formError = null;
    dialog = true;
  }

  function openEdit(m: MemoryRow) {
    editing = m;
    content = m.content;
    kind = m.kind;
    where = m.scope.startsWith("project:") ? "project" : m.scope.startsWith("conversation:") ? m.scope : "global";
    projectPath = m.scope.startsWith("project:") ? m.scope.slice(8) : "";
    subject = m.subject ?? "";
    until = m.valid_until ? new Date(m.valid_until).toISOString().slice(0, 10) : "";
    formError = null;
    dialog = true;
  }

  const chosenScope = $derived(where === "project" ? `project:${projectPath.trim()}` : where);

  async function save() {
    formError = null;
    if (where === "project" && !projectPath.trim()) {
      formError = "Write the project folder, or choose another scope.";
      return;
    }
    if (editing) {
      const ok = await store.changeFact(
        {
          id: editing.id,
          content: content !== editing.content ? content : undefined,
          subject: subject.trim() && subject.trim() !== (editing.subject ?? "") ? subject.trim() : undefined,
          clear_subject: !subject.trim() && !!editing.subject,
          valid_until: until && until !== (editing.valid_until ? new Date(editing.valid_until).toISOString().slice(0, 10) : "") ? until : undefined,
          clear_valid_until: !until && !!editing.valid_until,
          scope: chosenScope !== editing.scope ? chosenScope : undefined,
        },
        true,
      );
      if (ok) dialog = false;
      else formError = store.error;
    } else {
      const ok = await store.addFact({ content, kind, scope: chosenScope, subject: subject.trim() || undefined, valid_until: until || undefined }, true);
      if (ok) dialog = false;
      else formError = store.error;
    }
  }

  // Deleting names the runs that used the memory: what they already received cannot be taken back.
  let doomed = $state<MemoryRow | null>(null);
  let doomedRuns = $state<string[]>([]);
  let confirm = $state(false);

  async function askDelete(m: MemoryRow) {
    try {
      doomedRuns = await memoryUsage(m.id);
    } catch (e) {
      doomedRuns = [];
      store.error = errorText(e);
    }
    doomed = m;
    confirm = true;
  }

  async function reallyDelete() {
    if (doomed) await store.deleteFact(doomed.id, true);
    confirm = false;
    doomed = null;
  }

  const date = (ms: number) => new Date(ms).toISOString().slice(0, 10);
  const scopeTone = (s: string) => (s === "global" ? "accent" : s.startsWith("project:") ? "info" : s.startsWith("agent:") ? "ok" : "muted");
  // An agent's memory shows the agent's name, not its id.
  const scopeName = (m: MemoryRow) => {
    if (!m.scope.startsWith("agent:")) return m.scope_label;
    const card = agents.cards.find((c) => c.id === m.scope.slice(6));
    return card ? `${card.emoji} ${card.name}` : "an agent";
  };
</script>

<div class="mx-auto flex h-full max-w-4xl flex-col gap-5 overflow-y-auto p-8 [&>*]:shrink-0">
  <PageHeader
    title="Memory"
    subtitle="What ZORIX remembers between runs. Nothing is added without you: a run can only propose, and you approve."
    glow
    class="mb-1"
  >
    {#snippet actions()}
      {#if tab === "facts"}<Button variant="primary" icon="plus" onclick={openAdd}>Remember something</Button>{/if}
    {/snippet}
  </PageHeader>

  <Tabs
    tabs={[
      { value: "approve", label: "To approve", count: proposals },
      { value: "facts", label: "Facts and preferences", count: activeCount },
      { value: "history", label: "Past work" },
    ]}
    bind:value={tab}
    label="Memory"
  />

  {#if store.error}<Callout tone="err">{store.error}</Callout>{/if}

  {#if tab === "approve"}
    <MemoryProposals />
  {:else if tab === "history"}
    <History />
  {:else}
    <div class="flex flex-wrap items-center gap-3">
      <Input class="min-w-56 flex-1" bind:value={query} placeholder="Search what is remembered" aria-label="Search what is remembered" />
      <Segmented
        class="w-96"
        label="Scope"
        bind:value={scope}
        options={[
          { value: "all", label: "All" },
          { value: "global", label: "Global" },
          { value: "project", label: "Project" },
          { value: "conversation", label: "Chat" },
          { value: "agent", label: "Agent" },
        ]}
      />
      <label class="flex items-center gap-2 text-sm text-muted"><input type="checkbox" bind:checked={showOld} /> Show replaced and expired</label>
    </div>

    {#if rows.length === 0}
      <EmptyState
          icon="book"
          title={store.memory.length ? "Nothing matches" : "Nothing is remembered yet"}
          text={store.memory.length
            ? "Change the search or the scope."
            : "Add a fact or a preference yourself, or let a run propose one and approve it."}
        />
    {/if}

    <div class="space-y-3">
      {#each rows as m (m.id)}
        <Card class={m.status === "active" ? "" : "opacity-65"}>
          <div class="flex items-start justify-between gap-4">
            <div class="min-w-0 flex-1">
              <p class="selectable text-md leading-relaxed break-words">{m.content}</p>
              <div class="mt-2.5 flex flex-wrap items-center gap-1.5">
                <Badge tone="muted">{m.kind}</Badge>
                <Badge tone={scopeTone(m.scope)}>{scopeName(m)}</Badge>
                {#if m.subject}<Badge tone="muted">about: {m.subject}</Badge>{/if}
                {#if m.status === "replaced"}<Badge tone="warn" dot>replaced{m.replaced_by ? ` by ${m.replaced_by}` : ""}</Badge>{/if}
                {#if m.status === "expired"}<Badge tone="warn" dot>expired</Badge>{/if}
                {#if m.stale_days}<Badge tone="warn" dot>not confirmed for {m.stale_days} days</Badge>{/if}
                {#if m.valid_until && m.status === "active"}<Badge tone="muted">until {date(m.valid_until)}</Badge>{/if}
              </div>
              <p class="mt-2 text-xs text-faint">
                {m.by} · added {ago(m.created_at, store.now)} · confirmed {date(m.confirmed_at)} · used in {m.use_count} run(s)
                <span class="mono">{m.id}</span>
              </p>
            </div>
            {#if m.status === "active"}
              <div class="flex shrink-0 gap-1.5">
                {#if m.stale_days}<Button size="sm" icon="check" onclick={() => store.changeFact({ id: m.id, confirm: true }, true)}>Still true</Button>{/if}
                <Button size="sm" icon="edit" onclick={() => openEdit(m)}>Edit</Button>
                <Button size="sm" variant="danger" icon="trash" onclick={() => askDelete(m)}>Forget</Button>
              </div>
            {:else}
              <Button size="sm" variant="danger" icon="trash" onclick={() => askDelete(m)}>Forget</Button>
            {/if}
          </div>
        </Card>
      {/each}
    </div>
  {/if}
</div>

<Modal bind:open={dialog} title={editing ? "Edit memory" : "Remember something"} icon="book" size="lg">
  <div class="space-y-4">
    <Field label="What should be remembered? (one short sentence)">
      {#snippet children(f)}<Textarea id={f.id} rows={3} bind:value={content} maxlength={500} placeholder="For example: Deploys go to staging first." />{/snippet}
    </Field>
    <div class="grid grid-cols-2 gap-3">
      {#if !editing}
        <Field label="Kind">
          {#snippet children(f)}
            <Select id={f.id} bind:value={kind}>
              <option value="fact">Fact: something true</option>
              <option value="preference">Preference: how you want things done</option>
            </Select>
          {/snippet}
        </Field>
      {/if}
      <Field label="Where it applies">
        {#snippet children(f)}
          <Select id={f.id} bind:value={where}>
            <option value="global">Everywhere</option>
            <option value="project">One project folder</option>
            {#if editing?.scope.startsWith("conversation:")}<option value={editing.scope}>This conversation</option>{/if}
          </Select>
        {/snippet}
      </Field>
    </div>
    {#if where === "project"}
      <Field label="Project folder">
        {#snippet children(f)}<Input id={f.id} mono bind:value={projectPath} placeholder="/path/to/the/project" />{/snippet}
      </Field>
    {/if}
    <div class="grid grid-cols-2 gap-3">
      <Field label="About (optional): a newer memory about the same thing replaces this one">
        {#snippet children(f)}<Input id={f.id} bind:value={subject} maxlength={80} placeholder="for example: language" />{/snippet}
      </Field>
      <Field label="Forget after (optional)">
        {#snippet children(f)}<Input id={f.id} type="date" bind:value={until} />{/snippet}
      </Field>
    </div>
    <p class="text-xs text-faint">Do not store passwords or keys here: text that looks like one is refused.</p>
    {#if formError}<Callout tone="err">{formError}</Callout>{/if}
  </div>
  {#snippet footer()}
    <Button onclick={() => (dialog = false)}>Cancel</Button>
    <Button variant="primary" icon="check" loading={store.busy} disabled={!content.trim()} onclick={save}>{editing ? "Save" : "Remember"}</Button>
  {/snippet}
</Modal>

<Modal bind:open={confirm} title="Forget this?" icon="trash" size="md">
  {#if doomed}
    <p class="selectable text-md">{doomed.content}</p>
    <p class="mt-3 text-sm text-muted">It is deleted from everything ZORIX keeps, including the record of runs that used it.</p>
    {#if doomedRuns.length}
      <Callout tone="warn" class="mt-3">
        It was used in {doomedRuns.length} run(s). What those runs already received (their transcripts, and whatever a cloud provider
        received) cannot be taken back.
      </Callout>
    {/if}
  {/if}
  {#snippet footer()}
    <Button onclick={() => (confirm = false)}>Keep it</Button>
    <Button variant="danger" icon="trash" loading={store.busy} onclick={reallyDelete}>Forget</Button>
  {/snippet}
</Modal>
