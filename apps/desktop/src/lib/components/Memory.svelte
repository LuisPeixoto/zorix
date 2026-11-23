<script lang="ts">
  import { t } from "../i18n/index.ts";
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
      formError = t("memory.writeTheProjectFolder");
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
    return card ? `${card.emoji} ${card.name}` : t("memory.anAgent");
  };
</script>

<div class="mx-auto flex h-full max-w-4xl flex-col gap-5 overflow-y-auto p-8 [&>*]:shrink-0">
  <PageHeader
    title={t("common.memory")}
    subtitle={t("memory.whatZorixRemembersBetween")}
    glow
    class="mb-1"
  >
    {#snippet actions()}
      {#if tab === "facts"}<Button variant="primary" icon="plus" onclick={openAdd}>{t("memory.rememberSomething")}</Button>{/if}
    {/snippet}
  </PageHeader>

  <Tabs
    tabs={[
      { value: "approve", label: "To approve", count: proposals },
      { value: "facts", label: "Facts and preferences", count: activeCount },
      { value: "history", label: "Past work" },
    ]}
    bind:value={tab}
    label={t("common.memory")}
  />

  {#if store.error}<Callout tone="err">{store.error}</Callout>{/if}

  {#if tab === "approve"}
    <MemoryProposals />
  {:else if tab === "history"}
    <History />
  {:else}
    <div class="flex flex-wrap items-center gap-3">
      <Input class="min-w-56 flex-1" bind:value={query} placeholder={t("memory.searchWhatIsRemembered")} aria-label={t("memory.searchWhatIsRemembered")} />
      <Segmented
        class="w-96"
        label={t("memory.scope")}
        bind:value={scope}
        options={[
          { value: "all", label: "All" },
          { value: "global", label: "Global" },
          { value: "project", label: "Project" },
          { value: "conversation", label: "Chat" },
          { value: "agent", label: "Agent" },
        ]}
      />
      <label class="flex items-center gap-2 text-sm text-muted"><input type="checkbox" bind:checked={showOld} /> {t("memory.showReplacedAndExpired")}</label>
    </div>

    {#if rows.length === 0}
      <EmptyState
          icon="book"
          title={store.memory.length ? t("memory.nothingMatches") : t("memory.nothingIsRememberedYet")}
          text={store.memory.length
            ? t("memory.changeTheSearchOr")
            : t("memory.addAFactOr")}
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
                {#if m.stale_days}<Badge tone="warn" dot>{t("memory.notConfirmedForDays", { stale_days: m.stale_days })}</Badge>{/if}
                {#if m.valid_until && m.status === "active"}<Badge tone="muted">until {date(m.valid_until)}</Badge>{/if}
              </div>
              <p class="mt-2 text-xs text-faint">
                {t("memory.addedConfirmedUsedIn", { by: m.by, value: ago(m.created_at, store.now), value2: date(m.confirmed_at), use_count: m.use_count })}
                <span class="mono">{m.id}</span>
              </p>
            </div>
            {#if m.status === "active"}
              <div class="flex shrink-0 gap-1.5">
                {#if m.stale_days}<Button size="sm" icon="check" onclick={() => store.changeFact({ id: m.id, confirm: true }, true)}>{t("memory.stillTrue")}</Button>{/if}
                <Button size="sm" icon="edit" onclick={() => openEdit(m)}>{t("memory.edit")}</Button>
                <Button size="sm" variant="danger" icon="trash" onclick={() => askDelete(m)}>{t("memory.forget")}</Button>
              </div>
            {:else}
              <Button size="sm" variant="danger" icon="trash" onclick={() => askDelete(m)}>{t("memory.forget")}</Button>
            {/if}
          </div>
        </Card>
      {/each}
    </div>
  {/if}
</div>

<Modal bind:open={dialog} title={editing ? t("memory.editMemory") : t("memory.rememberSomething")} icon="book" size="lg">
  <div class="space-y-4">
    <Field label={t("memory.whatShouldBeRemembered")}>
      {#snippet children(f)}<Textarea id={f.id} rows={3} bind:value={content} maxlength={500} placeholder={t("memory.forExampleDeploysGo")} />{/snippet}
    </Field>
    <div class="grid grid-cols-2 gap-3">
      {#if !editing}
        <Field label={t("common.kind")}>
          {#snippet children(f)}
            <Select id={f.id} bind:value={kind}>
              <option value="fact">{t("memory.factSomethingTrue")}</option>
              <option value="preference">{t("memory.preferenceHowYouWant")}</option>
            </Select>
          {/snippet}
        </Field>
      {/if}
      <Field label={t("memory.whereItApplies")}>
        {#snippet children(f)}
          <Select id={f.id} bind:value={where}>
            <option value="global">{t("memory.everywhere")}</option>
            <option value="project">{t("memory.oneProjectFolder")}</option>
            {#if editing?.scope.startsWith("conversation:")}<option value={editing.scope}>{t("memory.thisConversation")}</option>{/if}
          </Select>
        {/snippet}
      </Field>
    </div>
    {#if where === "project"}
      <Field label={t("memory.projectFolder")}>
        {#snippet children(f)}<Input id={f.id} mono bind:value={projectPath} placeholder="/path/to/the/project" />{/snippet}
      </Field>
    {/if}
    <div class="grid grid-cols-2 gap-3">
      <Field label={t("memory.aboutOptionalANewer")}>
        {#snippet children(f)}<Input id={f.id} bind:value={subject} maxlength={80} placeholder={t("memory.forExampleLanguage")} />{/snippet}
      </Field>
      <Field label={t("memory.forgetAfterOptional")}>
        {#snippet children(f)}<Input id={f.id} type="date" bind:value={until} />{/snippet}
      </Field>
    </div>
    <p class="text-xs text-faint">{t("memory.doNotStorePasswords")}</p>
    {#if formError}<Callout tone="err">{formError}</Callout>{/if}
  </div>
  {#snippet footer()}
    <Button onclick={() => (dialog = false)}>{t("common.cancel")}</Button>
    <Button variant="primary" icon="check" loading={store.busy} disabled={!content.trim()} onclick={save}>{editing ? t("common.save") : t("memory.remember")}</Button>
  {/snippet}
</Modal>

<Modal bind:open={confirm} title={t("memory.forgetThis")} icon="trash" size="md">
  {#if doomed}
    <p class="selectable text-md">{doomed.content}</p>
    <p class="mt-3 text-sm text-muted">{t("memory.itIsDeletedFrom")}</p>
    {#if doomedRuns.length}
      <Callout tone="warn" class="mt-3">
        {t("memory.itWasUsedIn", { length: doomedRuns.length })}
      </Callout>
    {/if}
  {/if}
  {#snippet footer()}
    <Button onclick={() => (confirm = false)}>{t("common.keepIt")}</Button>
    <Button variant="danger" icon="trash" loading={store.busy} onclick={reallyDelete}>{t("memory.forget")}</Button>
  {/snippet}
</Modal>
