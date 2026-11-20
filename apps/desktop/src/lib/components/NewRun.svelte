<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { type AgentDetail, agentGet } from "../api.ts";
  import { agents } from "../stores/agents.svelte.ts";

  import { errorText } from "../api.ts";
  import { integrations } from "../stores/integrations.svelte.ts";
  import { store } from "../stores/runs.svelte.ts";
  import { Avatar, Badge, BrandLogo, Button, Callout, Card, Checkbox, Chip, Field, Icon, Input, PageHeader, Select, Switch, Textarea } from "../ui/index.ts";

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
  // Run as an agent: it brings its own instructions, tools and access, and this form can only narrow them.
  // One choice: nothing, an agent, or a team (`team:slug`), which runs its lead with the members to ask.
  let choice = $state("");
  const team = $derived(choice.startsWith("team:") ? agents.team(choice.slice(5)) : undefined);
  const agentSlug = $derived(team ? team.team.lead : choice.startsWith("team:") ? "" : choice);
  let agentDetail = $state<AgentDetail | null>(null);
  $effect(() => {
    const slug = agentSlug;
    agentDetail = null;
    if (slug) void agentGet(slug).then((d) => { if (agentSlug === slug) agentDetail = d; }).catch(() => {});
  });
  $effect(() => {
    if (!agents.loaded) void agents.refresh();
  });
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

  // What the run may reach beyond the built-in tools: MCP servers, databases, code, and hosts, repositories and channels by name.
  let pickedServers = $state<Record<string, boolean>>({});
  let pickedDbs = $state<Record<string, boolean>>({});
  let dbWrites = $state<Record<string, boolean>>({});
  let pickedCode = $state<Record<string, boolean>>({});
  let hosts = $state("");
  let repos = $state("");
  let jira = $state("");
  let chat = $state("");
  const builtinTools = $derived(store.tools.filter((t) => !t.source || t.source.kind === "builtin" || t.source.kind === "memory"));
  const readyServers = $derived(integrations.servers.filter((s) => s.enabled && s.tools_ready > 0));
  const databases = $derived(integrations.connections.filter((c) => c.kind === "sqlite" || c.kind === "postgres"));
  const reach = $derived(
    readyServers.length + databases.length > 0 || integrations.connections.length > 0 || integrations.servers.length > 0,
  );
  const names = (text: string) => text.split(/[\n,]/).map((s) => s.trim()).filter(Boolean);
  const chosen_ = (picked: Record<string, boolean>) => Object.entries(picked).filter(([, on]) => on).map(([k]) => k);

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
    if (agentSlug) {
      void store.startAgent({
        slug: agentSlug,
        team: team?.team.slug,
        prompt: prompt.trim(),
        model: model || undefined,
        conversation: continuing?.id,
        memory: { conversation: useConversation, long_term: useMemory, knowledge: useKnowledge, operations: useMemory },
      });
      return;
    }
    void store.start({
      prompt: prompt.trim(),
      allow_tools: builtinTools.map((t) => t.id).filter(isOn),
      integrations: {
        mcp: chosen_(pickedServers),
        db: chosen_(pickedDbs).map((n) => (dbWrites[n] ? `${n}:write` : n)),
        code: chosen_(pickedCode),
        http: names(hosts),
        repo: names(repos),
        jira: names(jira),
        chat: names(chat),
      },
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

  {#if agents.cards.length}
    <Card title="Run as" icon="bot">
      <Field label="Who does it?" hint={team ? `${team.lead?.name ?? team.team.lead} leads and may hand work to ${team.team.members.length} other agent${team.team.members.length === 1 ? "" : "s"}. Their cost and tokens count against this run.` : agentSlug ? "" : "Pick an agent or a team to use its instructions, skills, tools and own memory. Or leave a plain run and set everything below."}>
        {#snippet children(f)}
          <Select id={f.id} bind:value={choice}>
            <option value="">No agent: a plain run</option>
            <optgroup label="Agents">
              {#each agents.cards as c (c.slug)}<option value={c.slug}>{c.emoji} {c.name} · v{c.version}</option>{/each}
            </optgroup>
            {#if agents.teams.length}
              <optgroup label="Teams">
                {#each agents.teams as t (t.team.slug)}<option value={`team:${t.team.slug}`}>{t.team.avatar.emoji} {t.team.name} · {t.team.members.length + 1} agents</option>{/each}
              </optgroup>
            {/if}
          </Select>
        {/snippet}
      </Field>
      {#if agentDetail}
        {@const spec = agentDetail.version.spec}
        <div class="mt-4 flex items-start gap-3 rounded-xl border border-line bg-panel-2 p-3">
          <Avatar emoji={spec.avatar.emoji} color={spec.avatar.color} size="lg" />
          <div class="min-w-0 flex-1">
            <p class="text-md font-semibold">{spec.name} <span class="text-sm font-normal text-faint">v{agentDetail.agent.current_version}</span></p>
            <p class="text-sm text-muted">{spec.description}</p>
            <p class="mt-2 flex flex-wrap gap-1.5">
              {#each spec.tools as t (t)}<Badge tone="muted" class="mono">{t}</Badge>{/each}
              {#each spec.access as a (a)}<Badge tone="accent" class="mono">{a}</Badge>{/each}
              {#each spec.skills as k (k.name)}<Badge tone="info">skill {k.name}</Badge>{/each}
              <Badge tone="muted">up to {spec.limits.max_iterations} steps</Badge>
            </p>
            <p class="mt-2 text-xs text-faint">It keeps all of this. Changing the model or switching memory off below only narrows it; it never adds. Anything that changes something still asks you first.</p>
          </div>
        </div>
      {/if}
    </Card>
  {/if}

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
          <option value="">Default ({agentDetail ? agentDetail.version.spec.models.default : defaultName})</option>
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

  {#if !agentSlug}
  <Card title="Tools it may use" icon="wrench">
    <div class="grid gap-2 sm:grid-cols-2">
      {#each builtinTools as tool (tool.id)}
        <div class="rounded-xl border p-3 transition duration-150 {isOn(tool.id) ? 'border-accent/45 bg-accent-soft' : 'border-line bg-panel hover:border-line-strong'}">
          <Checkbox checked={isOn(tool.id)} onchange={(e) => (overrides[tool.id] = e.currentTarget.checked)} description={tool.description}>
            <span class="mono text-sm font-medium">{tool.id}</span>
            <Badge tone={howTone(tool)} class="ml-1.5 h-5">{how(tool)}</Badge>
          </Checkbox>
        </div>
      {/each}
    </div>
  </Card>

  <Card title="Servers and services" icon="plug">
    {#if !reach}
      <p class="text-sm text-muted">Nothing is connected yet. <button type="button" class="text-accent underline" onclick={() => store.showIntegrations("catalogue")}>Connect a server or a service</button> to let a run use your repositories, tickets, databases and more.</p>
    {:else}
      <p class="mb-3 text-sm text-muted">A run reaches only what you pick here. Each MCP tool still asks you first; databases are read-only unless you allow writes, and writes ask too.</p>
      {#if readyServers.length}
        <p class="mb-1.5 text-xs font-medium text-muted">MCP servers</p>
        <div class="mb-4 flex flex-wrap gap-2">
          {#each readyServers as server (server.name)}
            <button
              type="button"
              aria-pressed={!!pickedServers[server.name]}
              onclick={() => (pickedServers[server.name] = !pickedServers[server.name])}
              class="flex items-center gap-2 rounded-xl border px-2.5 py-1.5 text-left transition duration-150 {pickedServers[server.name] ? 'border-accent/50 bg-accent-soft' : 'border-line bg-panel hover:border-line-strong'}"
            >
              <BrandLogo name={server.logo ?? (server.kind === "http" ? "globe" : "server")} size={28} />
              <span><span class="block text-base font-medium">{server.name}</span><span class="tnum block text-xs text-muted">{server.tools_ready} tools ready</span></span>
              {#if pickedServers[server.name]}<Icon name="check" size={14} class="text-accent" />{/if}
            </button>
          {/each}
        </div>
      {/if}
      {#if databases.length}
        <p class="mb-1.5 text-xs font-medium text-muted">Databases</p>
        <div class="mb-4 space-y-2">
          {#each databases as db (db.id)}
            <div class="flex flex-wrap items-center gap-3 rounded-xl border px-3 py-2 {pickedDbs[db.name] ? 'border-accent/50 bg-accent-soft' : 'border-line bg-panel'}">
              <Checkbox checked={!!pickedDbs[db.name]} onchange={(e) => (pickedDbs[db.name] = e.currentTarget.checked)}>
                <BrandLogo name={db.kind} size={24} class="mr-1 inline-grid align-middle" /> <span class="mono text-sm font-medium">{db.name}</span>
              </Checkbox>
              {#if pickedDbs[db.name]}<Switch bind:checked={dbWrites[db.name]} class="ml-auto">Allow writes (each asks first)</Switch>{/if}
            </div>
          {/each}
        </div>
      {/if}
      <p class="mb-1.5 text-xs font-medium text-muted">Code</p>
      <div class="mb-4 flex flex-wrap gap-2">
        <Chip selected={!!pickedCode.javascript} onclick={() => (pickedCode.javascript = !pickedCode.javascript)}>Run JavaScript</Chip>
        <Chip selected={!!pickedCode.python} onclick={() => (integrations.sandbox?.python ? (pickedCode.python = !pickedCode.python) : store.showIntegrations("connections"))}>
          Run Python{integrations.sandbox?.python ? "" : " (install it first)"}
        </Chip>
      </div>
      <div class="grid gap-3 sm:grid-cols-2">
        <Field label="Web hosts" hint="Comma separated, for example api.example.com">
          {#snippet children(f)}<Input id={f.id} mono bind:value={hosts} placeholder="none" />{/snippet}
        </Field>
        <Field label="Repositories" hint="owner/repo, or owner/repo:write to also write">
          {#snippet children(f)}<Input id={f.id} mono bind:value={repos} placeholder="none" />{/snippet}
        </Field>
        <Field label="Jira projects" hint="KEY, or KEY:write">
          {#snippet children(f)}<Input id={f.id} mono bind:value={jira} placeholder="none" />{/snippet}
        </Field>
        <Field label="Chat channels" hint="slack:#alerts, discord:<channel id>, add :write to send">
          {#snippet children(f)}<Input id={f.id} mono bind:value={chat} placeholder="none" />{/snippet}
        </Field>
      </div>
    {/if}
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

  {/if}

  {#if store.error}<Callout tone="err">{store.error}</Callout>{/if}

  <div class="sticky -bottom-8 z-20 -mx-8 -mb-8 flex items-center justify-end gap-3 border-t border-line bg-bg/90 px-8 py-4 backdrop-blur">
    <Button variant="primary" size="lg" icon="play" type="submit" loading={store.busy} disabled={!prompt.trim()}>{team ? "Start with the team" : agentSlug ? "Start with the agent" : "Start run"}</Button>
  </div>
</form>
