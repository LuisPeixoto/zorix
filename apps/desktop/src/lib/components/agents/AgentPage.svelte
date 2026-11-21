<script lang="ts">
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

  const labels: Record<AgentTab, string> = { chat: "Chat", definition: "Definition", skills: "Skills", versions: "Versions", evals: "Evals", memory: "Memory" };
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
      toast.ok(`${detail.agent.slug} is archived. Its history and memory are kept.`);
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
      toast.ok(`${detail.agent.slug} was deleted.`);
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
      toast.ok("The agent file is copied.");
    } catch {
      toast.info("Select the text and copy it.");
    }
  }
</script>

<div class="flex h-full min-h-0 flex-col">
  <header class="relative shrink-0 border-b border-line bg-panel/80 px-8 pt-5 backdrop-blur">
    <button type="button" class="mb-3 inline-flex items-center gap-1 text-sm text-muted transition hover:text-fg" onclick={() => agents.open(null)}>
      <Icon name="chevron-right" size={14} class="rotate-180" /> All agents
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
          <p class="mt-1.5 max-w-3xl text-md text-muted">{spec.description || "No description yet. Add one on the Definition tab."}</p>
        </div>
        <div class="flex shrink-0 items-center gap-1.5">
          {#if agents.tab !== "evals"}<Button size="sm" icon="flask" onclick={() => (tab = "evals")}>Run evals</Button>{/if}
          {#if agents.tab !== "chat"}<Button size="sm" variant="subtle" icon="message" onclick={() => (tab = "chat")}>Chat</Button>{/if}
          <Button size="sm" icon="file-text" onclick={showFile}>Agent file</Button>
          <IconButton icon="archive" label="Archive this agent" onclick={archive} />
          <IconButton icon="trash" label="Delete this agent" onclick={() => (confirmOpen = true)} />
        </div>
      </div>
    {:else if agents.error}
      <Callout tone="err" title="This agent could not be opened">{agents.error}</Callout>
    {:else}
      <div class="flex gap-4"><Skeleton class="size-16 rounded-2xl" /><div class="flex-1 space-y-2"><Skeleton class="h-7 w-64" /><Skeleton class="h-4 w-96" /></div></div>
    {/if}
    <Tabs {tabs} bind:value={tab} label="Agent sections" class="mt-4 -mb-px" />
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

<Modal open={exported !== null} onclose={() => (exported = null)} title="Agent file" icon="file-text" size="lg">
  {#if exported !== null}
    <p class="mb-3 text-sm text-muted">This is the whole agent as one Markdown file. Keep it in git, or import it on another computer with <span class="mono">zorix agent import</span>.</p>
    <pre class="mono selectable max-h-[55vh] overflow-auto rounded-xl border border-line bg-panel-2 p-3 text-sm whitespace-pre-wrap">{exported}</pre>
  {/if}
  {#snippet footer()}
    <Button onclick={() => (exported = null)}>Close</Button>
    <Button variant="primary" icon="copy" onclick={copyFile}>Copy</Button>
  {/snippet}
</Modal>

<Modal bind:open={confirmOpen} title="Delete this agent?" icon="trash" size="sm">
  <p class="text-base text-muted">
    <strong class="text-fg">{spec?.name}</strong> is deleted with all its versions and evaluation cases. Runs it made stay in the list. What it remembered stays until you forget it.
    To only hide it, use Archive.
  </p>
  {#snippet footer()}
    <Button onclick={() => (confirmOpen = false)}>Cancel</Button>
    <Button variant="danger" icon="trash" loading={removing} onclick={remove}>Delete</Button>
  {/snippet}
</Modal>
