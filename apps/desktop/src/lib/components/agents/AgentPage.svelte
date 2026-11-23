<script lang="ts">
  import { t } from "../../i18n/index.ts";
  import { agentArchive, agentExport, agentRemove, errorText } from "../../api.ts";
  import { AGENT_TABS, type AgentTab, agents } from "../../stores/agents.svelte.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { Avatar, Badge, Button, Callout, Icon, IconButton, Modal, Skeleton, Tabs } from "../../ui/index.ts";
  import AgentChat from "./AgentChat.svelte";
  import AgentDefinition from "./AgentDefinition.svelte";
  import AgentEvals from "./AgentEvals.svelte";
  import AgentMemory from "./AgentMemory.svelte";
  import AgentSkills from "./AgentSkills.svelte";
  import AgentVersions from "./AgentVersions.svelte";

  const detail = $derived(agents.detail);
  const spec = $derived(detail?.version.spec);
  let removing = $state(false);
  let confirmOpen = $state(false);
  let exported = $state<string | null>(null);

  const labels: Record<AgentTab, string> = { chat: t("agents.agentPage.chat"), definition: t("agents.agentPage.definition"), skills: t("agents.agentPage.skills"), versions: t("common.versions"), evals: t("agents.agentPage.evals"), memory: t("common.memory") };
  const tabs = $derived(
    AGENT_TABS.map((value) => ({
      value,
      label: labels[value],
      count: value === "skills" ? detail?.version.spec.skills.length || undefined : value === "evals" ? detail?.cases || undefined : undefined,
    })),
  );
  let tab = $state<string>(agents.tab);
  $effect(() => {
    if (tab !== agents.tab) agents.tab = tab as AgentTab;
  });
  $effect(() => {
    tab = agents.tab;
  });

  async function archive() {
    if (!detail) return;
    try {
      await agentArchive(detail.agent.slug, true);
      toast.ok(t("agents.agentPage.isArchivedItsHistory", { slug: detail.agent.slug }));
      await agents.open(null);
    } catch (e) {
      toast.err(errorText(e));
    }
  }

  async function remove() {
    if (!detail) return;
    removing = true;
    try {
      await agentRemove(detail.agent.slug);
      toast.ok(t("agents.agentPage.wasDeleted", { slug: detail.agent.slug }));
      confirmOpen = false;
      await agents.open(null);
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      removing = false;
    }
  }

  async function showFile() {
    if (!detail) return;
    try {
      exported = await agentExport(detail.agent.slug);
    } catch (e) {
      toast.err(errorText(e));
    }
  }
  async function copyFile() {
    if (!exported) return;
    try {
      await navigator.clipboard.writeText(exported);
      toast.ok(t("agents.agentPage.theAgentFileIs"));
    } catch {
      toast.info(t("agents.agentPage.selectTheTextAnd"));
    }
  }
</script>

<div class="flex h-full min-h-0 flex-col">
  <header class="relative shrink-0 border-b border-line bg-panel/80 px-8 pt-5 backdrop-blur">
    <button type="button" class="mb-3 inline-flex items-center gap-1 text-sm text-muted transition hover:text-fg" onclick={() => agents.open(null)}>
      <Icon name="chevron-right" size={14} class="rotate-180" /> {t("agents.agentPage.allAgents")}
    </button>
    {#if spec && detail}
      <div class="flex items-start gap-4">
        <Avatar emoji={spec.avatar.emoji} color={spec.avatar.color} size="xl" label={spec.name} />
        <div class="min-w-0 flex-1">
          <div class="flex flex-wrap items-center gap-2.5">
            <h1 class="text-3xl font-semibold tracking-tight">{spec.name}</h1>
            <Badge tone="accent">v{detail.agent.current_version}</Badge>
            <Badge tone="muted">{spec.models.default}</Badge>
          </div>
          <p class="mono mt-0.5 text-xs text-faint">{detail.agent.slug}</p>
          <p class="mt-1.5 max-w-3xl text-md text-muted">{spec.description || t("agents.agentPage.noDescriptionYetAdd")}</p>
        </div>
        <div class="flex shrink-0 items-center gap-1.5">
          {#if agents.tab !== "evals"}<Button size="sm" icon="flask" onclick={() => (tab = "evals")}>{t("agents.agentPage.runEvals")}</Button>{/if}
          {#if agents.tab !== "chat"}<Button size="sm" variant="subtle" icon="message" onclick={() => (tab = "chat")}>{t("agents.agentPage.chat")}</Button>{/if}
          <Button size="sm" icon="file-text" onclick={showFile}>{t("agents.agentPage.agentFile")}</Button>
          <IconButton icon="archive" label={t("agents.agentPage.archiveThisAgent")} onclick={archive} />
          <IconButton icon="trash" label={t("agents.agentPage.deleteThisAgent")} onclick={() => (confirmOpen = true)} />
        </div>
      </div>
    {:else if agents.error}
      <Callout tone="err" title={t("agents.agentPage.thisAgentCouldNot")}>{agents.error}</Callout>
    {:else}
      <div class="flex gap-4"><Skeleton class="size-16 rounded-2xl" /><div class="flex-1 space-y-2"><Skeleton class="h-7 w-64" /><Skeleton class="h-4 w-96" /></div></div>
    {/if}
    <Tabs {tabs} bind:value={tab} label={t("agents.agentPage.agentSections")} class="mt-4 -mb-px" />
  </header>

  <div class="min-h-0 flex-1">
    {#if detail}
      {#if agents.tab === "chat"}<AgentChat {detail} />
      {:else if agents.tab === "definition"}<AgentDefinition {detail} />
      {:else if agents.tab === "skills"}<AgentSkills {detail} />
      {:else if agents.tab === "versions"}<AgentVersions {detail} />
      {:else if agents.tab === "evals"}<AgentEvals {detail} />
      {:else}<AgentMemory {detail} />
      {/if}
    {/if}
  </div>
</div>

<Modal open={exported !== null} onclose={() => (exported = null)} title={t("agents.agentPage.agentFile")} icon="file-text" size="lg">
  {#if exported !== null}
    <p class="mb-3 text-sm text-muted">{t("agents.agentPage.thisIsTheWhole")} <span class="mono">{t("agents.agentPage.zorixAgentImport")}</span>.</p>
    <pre class="mono selectable max-h-[55vh] overflow-auto rounded-xl border border-line bg-panel-2 p-3 text-sm whitespace-pre-wrap">{exported}</pre>
  {/if}
  {#snippet footer()}
    <Button onclick={() => (exported = null)}>{t("common.close")}</Button>
    <Button variant="primary" icon="copy" onclick={copyFile}>{t("agents.agentPage.copy")}</Button>
  {/snippet}
</Modal>

<Modal bind:open={confirmOpen} title={t("agents.agentPage.deleteThisAgent2")} icon="trash" size="sm">
  <p class="text-base text-muted">
    <strong class="text-fg">{spec?.name}</strong> {t("agents.agentPage.isDeletedWithAll")}
  </p>
  {#snippet footer()}
    <Button onclick={() => (confirmOpen = false)}>{t("common.cancel")}</Button>
    <Button variant="danger" icon="trash" loading={removing} onclick={remove}>{t("common.delete")}</Button>
  {/snippet}
</Modal>
