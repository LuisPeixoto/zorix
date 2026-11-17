<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";

  import { errorText } from "../api.ts";
  import { store } from "../stores/runs.svelte.ts";
  import { Badge, Button, Callout, Card, Checkbox, Chip, Field, Icon, Input, PageHeader, Select, Switch, Textarea } from "../ui/index.ts";

  const examples = [
    "What is 17*23 and save it as result",
    "What time is it?",
    "Recall result",
    'Count the words in "the quick brown fox"',
  ];

  let prompt = $state("");
  let workspace = $state("");
  let allowRead = $state("*");
  let allowWrite = $state("");
  // "" means: whatever the default is (shown in the first option).
  let model = $state("");
  // What the person changed from the defaults. Offer the tools that need nothing more than a tool permission; file tools
  // need a workspace, so they start off.
  let overrides = $state<Record<string, boolean>>({});
  let pickError = $state<string | null>(null);
  // What the run may use from memory. All on by default; each can be switched off for one run.
  let useConversation = $state(true);
  let useMemory = $state(true);
  let useKnowledge = $state(true);
  // Continuing a conversation from the Run view (the Continue button) or starting a new one.
  const continuing = $derived(store.continuing);

  const isOn = (id: string) => overrides[id] ?? !id.startsWith("workspace.");

  const how = (t: { requires_approval: boolean; annotations: { read_only: boolean } }) =>
    t.requires_approval ? "asks first" : t.annotations.read_only ? "read-only" : "changes things";
  const howTone = (t: { requires_approval: boolean; annotations: { read_only: boolean } }) =>
    t.requires_approval ? "warn" : t.annotations.read_only ? "ok" : "err";

  const defaultName = $derived(store.models && "Ok" in store.models.default ? store.models.default.Ok : "the built-in demo model");
  const chosen = $derived(store.models?.models.find((m) => m.reference === model));
  const goesToCloud = $derived(!!model && model !== "demo" && !chosen?.local);

  async function pickWorkspace() {
    pickError = null;
    try {
      const folder = await open({ directory: true, multiple: false, title: "Choose the workspace folder" });
      if (typeof folder === "string") workspace = folder;
    } catch (e) {
      pickError = errorText(e);
    }
  }

  const lines = (text: string) => text.split(/[\n,]/).map((s) => s.trim()).filter(Boolean);

  function submit(event: Event) {
    event.preventDefault();
    void store.start({
      prompt: prompt.trim(),
      allow_tools: store.tools.map((t) => t.id).filter(isOn),
      allow_read: workspace ? lines(allowRead) : [],
      allow_write: workspace ? lines(allowWrite) : [],
      workspace: workspace || undefined,
      model: model || undefined,
      conversation: continuing?.id,
      memory: { conversation: useConversation, long_term: useMemory, knowledge: useKnowledge, operations: useMemory },
    });
  }
</script>

<form class="mx-auto flex h-full max-w-3xl flex-col gap-5 overflow-y-auto p-8 [&>*]:shrink-0" onsubmit={submit}>
  <PageHeader
    title="New run"
    subtitle="Say what you want done. The run can only use what you allow below, and anything that changes something asks you first."
    glow
    class="mb-1"
  />

  <Card title="What should it do?" icon="sparkles">
    <Textarea id="prompt" rows={4} bind:value={prompt} maxlength={20000} placeholder="For example: What is 17*23 and save it as result" aria-label="What should it do?" />
    <div class="mt-3 flex flex-wrap gap-2">
      {#each examples as example}
        <Chip onclick={() => (prompt = example)}>{example}</Chip>
      {/each}
    </div>
  </Card>

  <Card title="Memory" icon="book">
    {#if continuing}
      <Callout tone="info" icon="history">
        Continuing a conversation that began with “{continuing.prompt.length > 70 ? continuing.prompt.slice(0, 70) + "…" : continuing.prompt}”.
        <button type="button" class="ml-1 underline" onclick={() => (store.continuing = null)}>Start a new one instead</button>
      </Callout>
    {:else}
      <p class="text-sm text-muted">This starts a new conversation. Use Continue on a finished run to add to one.</p>
    {/if}
    <div class="mt-3 space-y-2.5">
      <Switch bind:checked={useConversation}>Earlier runs of this conversation</Switch>
      <Switch bind:checked={useMemory}>Things you asked it to remember, and past work</Switch>
      <Switch bind:checked={useKnowledge}>Indexed documents</Switch>
    </div>
    <p class="mt-3 text-xs text-faint">What is used is chosen once, when the run starts, and shown on the run under Context.</p>
  </Card>

  <Card title="Model" icon="cpu">
    <Field label="Which model answers?">
      {#snippet children(f)}
        <Select id={f.id} bind:value={model}>
          <option value="">Default ({defaultName})</option>
          <option value="demo">demo (built-in, no network)</option>
          {#each store.models?.routes ?? [] as r}<option value={"route:" + r}>route: {r}</option>{/each}
          {#each store.models?.aliases ?? [] as a}<option value={a}>alias: {a}</option>{/each}
          {#each store.models?.models ?? [] as m (m.reference)}
            <option value={m.reference}>{m.reference}{m.local ? " · on this computer" : ""}</option>
          {/each}
        </Select>
      {/snippet}
    </Field>
    <div class="mt-3">
      {#if goesToCloud}
        <Callout tone="warn" icon="cloud">The conversation, including tool results, is sent to the provider.</Callout>
      {:else if model && model !== "demo"}
        <Badge tone="ok" dot>Stays on this computer</Badge>
      {:else if store.models && "Err" in store.models.default && !model}
        <p class="flex items-center gap-1.5 text-sm text-muted"><Icon name="info" size={14} /> No provider is set up yet, so runs use the demo model. Add a key under Providers &amp; keys.</p>
      {/if}
    </div>
  </Card>

  <Card title="Tools it may use" icon="wrench">
    <div class="grid gap-2 sm:grid-cols-2">
      {#each store.tools as tool (tool.id)}
        <div class="rounded-xl border p-3 transition duration-150 {isOn(tool.id) ? 'border-accent/45 bg-accent-soft' : 'border-line bg-panel hover:border-line-strong'}">
          <Checkbox checked={isOn(tool.id)} onchange={(e) => (overrides[tool.id] = e.currentTarget.checked)} description={tool.description}>
            <span class="mono text-sm font-medium">{tool.id}</span>
            <Badge tone={howTone(tool)} class="ml-1.5 h-5">{how(tool)}</Badge>
          </Checkbox>
        </div>
      {/each}
    </div>
  </Card>

  <Card title="Workspace folder" icon="folder">
    <p class="mb-3 text-sm text-muted">The file tools are confined to this folder. With none, they refuse every call.</p>
    <div class="flex gap-2">
      <Input class="min-w-0 flex-1" mono bind:value={workspace} placeholder="No folder chosen" aria-label="Workspace folder" />
      <Button icon="folder" onclick={pickWorkspace}>Choose…</Button>
    </div>
    {#if pickError}<p class="mt-2 text-sm text-err" role="alert">{pickError}</p>{/if}
    {#if workspace}
      <div class="mt-4 grid grid-cols-2 gap-3">
        <Field label="May read (patterns)">
          {#snippet children(f)}<Input id={f.id} mono bind:value={allowRead} placeholder="* or docs/*" />{/snippet}
        </Field>
        <Field label="May write (patterns)">
          {#snippet children(f)}<Input id={f.id} mono bind:value={allowWrite} placeholder="none, or out/*" />{/snippet}
        </Field>
      </div>
    {/if}
  </Card>

  {#if store.error}<Callout tone="err">{store.error}</Callout>{/if}

  <div class="sticky bottom-0 -mx-8 -mb-8 flex items-center justify-end gap-3 border-t border-line bg-bg/90 px-8 py-4 backdrop-blur">
    <Button variant="primary" size="lg" icon="play" type="submit" loading={store.busy} disabled={!prompt.trim()}>Start run</Button>
  </div>
</form>
