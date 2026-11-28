<script lang="ts">
  import { fmt, t as tr } from "../i18n/index.ts";
  import { open } from "@tauri-apps/plugin-dialog";
  import { onDestroy, onMount, untrack } from "svelte";
  import { type AgentDetail, type RunEstimate, agentGet, estimateRun, workflowCreate, workflowSave, workflowStart } from "../api.ts";
  import { AttachedFiles } from "../stores/attachments.svelte.ts";
  import AtMenu from "./attachments/AtMenu.svelte";
  import AttachedList from "./attachments/AttachedList.svelte";
  import { workflows } from "../stores/workflows.svelte.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import { slugify } from "../workflow.ts";
  import { agents } from "../stores/agents.svelte.ts";

  import { errorText } from "../api.ts";
  import { integrations } from "../stores/integrations.svelte.ts";
  import { store } from "../stores/runs.svelte.ts";
  import { ago, duration, shorten } from "../format.ts";
  import { destination, grantSummary, providerName } from "../newRun.ts";
  import { Avatar, Badge, BrandLogo, Button, Callout, Card, Checkbox, Chip, Field, Icon, Input, KeyValue, Kbd, Modal, PageHeader, Segmented, Select, Switch, Textarea } from "../ui/index.ts";
  import StatusBadge from "./StatusBadge.svelte";

  const examples = [
    tr("newRun.whatIs1723"),
    tr("newRun.whatTimeIsIt"),
    tr("newRun.recallResult"),
    tr("newRun.countTheWordsIn"),
  ];

  let prompt = $state("");
  let workspace = $state("");
  // Files the run is given to read: chosen with the button, with `@` in the prompt, or dropped on the window (ADR-0049).
  const attached = new AttachedFiles(() => workspace.trim() || undefined);
  let promptField = $state<HTMLTextAreaElement | undefined>();
  let stopDrops: (() => void) | null = null;
  onMount(async () => {
    promptField = document.getElementById("prompt") as HTMLTextAreaElement | undefined;
    stopDrops = await attached.listen();
  });
  onDestroy(() => stopDrops?.());
  $effect(() => {
    void workspace;
    // Asked without following the files it reads: this runs when the workspace changes, not whenever a file is added.
    untrack(() => void attached.recheck());
  });
  let allowRead = $state("*");
  let allowWrite = $state("");
  // "" means: whatever the default is (shown in the first option).
  let model = $state("");
  // Run as an agent: it brings its own instructions, tools and access, and this form can only narrow them.
  // One choice: nothing, an agent, or a team (`team:slug`), which runs its lead with the members to ask.
  let choice = $state("");
  const team = $derived(choice.startsWith("team:") ? agents.team(choice.slice(5)) : undefined);
  const agentSlug = $derived(team ? team.team.lead : choice.startsWith("team:") || choice.startsWith("workflow:") ? "" : choice);
  // Workflows with a manual start can be run from here: what you write is what the workflow receives as `trigger.payload.prompt`.
  const workflowChoices = $derived((workflows.overview?.cards ?? []).filter((c) => c.workflow.live_version && c.triggers.some((t) => t.kind === "manual")));
  const picked = $derived(choice.startsWith("workflow:") ? workflowChoices.find((c) => c.workflow.id === choice.slice(9)) : undefined);
  $effect(() => {
    void workflows.refresh();
  });
  let savingAs = $state(false);
  let workflowName = $state("");
  let savingBusy = $state(false);
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

  // The limits of the run. Empty means the default (or, for an agent, its own).
  let budget = $state("");
  let minutes = $state("");
  let steps = $state("");
  const num = (text: string) => (text.trim() === "" ? null : Number(text));
  const budgetOk = $derived(num(budget) === null || (Number.isFinite(num(budget)!) && num(budget)! > 0));
  const minutesOk = $derived(num(minutes) === null || (Number.isInteger(num(minutes)!) && num(minutes)! >= 1 && num(minutes)! <= 1440));
  const stepsOk = $derived(num(steps) === null || (Number.isInteger(num(steps)!) && num(steps)! >= 1 && num(steps)! <= 1000));
  const limitsOk = $derived(budgetOk && minutesOk && stepsOk);
  const limits = $derived({
    ...(num(budget) !== null && budgetOk ? { max_cost_micros: Math.round(num(budget)! * 1_000_000) } : {}),
    ...(num(minutes) !== null && minutesOk ? { max_seconds: num(minutes)! * 60 } : {}),
    ...(num(steps) !== null && stepsOk ? { max_iterations: num(steps)! } : {}),
  });

  // What the run would probably cost and how long it would take, from what similar runs took. Asked a moment after typing stops.
  let estimate = $state<RunEstimate | null>(null);
  $effect(() => {
    const [chars, wanted, slug, files] = [prompt.length, model, agentSlug, attached.chars];
    estimate = null;
    if (mode === "workflow") return;
    const timer = setTimeout(() => {
      void estimateRun(chars, files, wanted, slug).then((e) => (estimate = e)).catch(() => (estimate = null));
    }, 300);
    return () => clearTimeout(timer);
  });
  const expected = $derived(
    estimate === null
      ? null
      : `${estimate.cost_micros === null ? tr("newRun.costUnknown") : `≈ ${fmt.usd2(estimate.cost_micros)}`} · ${duration(estimate.active_ms)}`,
  );
  const expectedBasis = $derived(
    estimate === null
      ? ""
      : estimate.basis.kind === "history"
        ? tr("newRun.fromYourLastSimilarRuns", { count: estimate.basis.runs })
        : tr("newRun.aRoughGuessFewerThan"),
  );

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
    t.requires_approval ? tr("newRun.asksFirst") : t.annotations.read_only ? "read-only" : tr("newRun.changesThings");
  const howTone = (t: { requires_approval: boolean; annotations: { read_only: boolean } }) =>
    t.requires_approval ? "warn" : t.annotations.read_only ? "ok" : "err";

  const defaultName = $derived(store.models && "Ok" in store.models.default ? store.models.default.Ok : tr("newRun.theBuiltInDemo"));
  const chosen = $derived(store.models?.models.find((m) => m.reference === model));
  const goesToCloud = $derived(!!model && model !== "demo" && !chosen?.local);

  async function pickWorkspace() {
    pickError = null;
    try {
      const folder = await open({ directory: true, multiple: false, title: tr("newRun.chooseTheWorkspaceFolder") });
      if (typeof folder === "string") workspace = folder;
    } catch (e) {
      pickError = errorText(e);
    }
  }

  // Run as: a plain run, an agent, or a team. `choice` keeps the three apart (empty, a slug, `team:slug`).
  const mode = $derived(choice.startsWith("team:") ? "team" : choice.startsWith("workflow:") ? "workflow" : choice ? "agent" : "plain");
  function setMode(next: string) {
    if (next === "plain") choice = "";
    else if (next === "agent") choice = agents.cards[0]?.slug ?? "";
    else if (next === "workflow") choice = workflowChoices[0] ? `workflow:${workflowChoices[0].workflow.id}` : "";
    else choice = agents.teams[0] ? `team:${agents.teams[0].team.slug}` : "";
  }
  const where = $derived(
    destination(model, store.models && "Ok" in store.models.default ? store.models.default.Ok : null, store.models?.models ?? []),
  );
  // What this run may use, for the "Before you start" summary.
  const grantedTools = $derived(
    agentSlug && agentDetail
      ? agentDetail.version.spec.tools.map((id) => store.tools.find((t) => t.id === id)).filter((t): t is NonNullable<typeof t> => !!t)
      : builtinTools.filter((t) => isOn(t.id)),
  );
  const grants = $derived(grantSummary(grantedTools));
  const recent = $derived(
    store.runs.filter((r) => (team ? r.agent?.team === team.team.slug : agentSlug ? r.agent?.slug === agentSlug : false)).slice(0, 3),
  );
  const memoryOn = $derived([useConversation && "conversation", useMemory && "memory", useKnowledge && "documents"].filter(Boolean).join(", ") || "none");
  const shortcut = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform || navigator.userAgent) ? "⌘↵" : tr("newRun.ctrl");
  function onkey(event: KeyboardEvent) {
    if ((event.metaKey || event.ctrlKey) && event.key === "Enter" && prompt.trim() && !store.busy) submit(event);
  }

  const lines = (text: string) => text.split(/[\n,]/).map((s) => s.trim()).filter(Boolean);

  async function saveAsWorkflow() {
    const slug = slugify(workflowName);
    if (!slug) return;
    savingBusy = true;
    try {
      const made = await workflowCreate(slug, workflowName.trim());
      const spec = structuredClone($state.snapshot(made.draft));
      const kind = team
        ? { type: "team" as const, team: team.team.slug, prompt: prompt.trim() }
        : { type: "agent" as const, agent: agentSlug, prompt: prompt.trim(), ...(model ? { model } : {}) };
      spec.steps.push({ id: team ? "team" : "agent", title: team ? tr("newRun.ask", { name: team.team.name }) : tr("newRun.ask2", { agentSlug: agentDetail?.version.spec.name ?? agentSlug }), x: 96, y: 176, kind });
      spec.edges.push({ from: spec.steps[0].id, to: spec.steps[1].id });
      await workflowSave(made.id, spec);
      savingAs = false;
      workflowName = "";
      toast.ok(tr("newRun.savedAsAWorkflow"));
      store.showWorkflows("editor", made.id);
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      savingBusy = false;
    }
  }

  function submit(event: Event) {
    event.preventDefault();
    if (picked) {
      void (async () => {
        try {
          const manual = picked.triggers.find((t) => t.kind === "manual");
          const run = await workflowStart(picked.workflow.id, false, { prompt: prompt.trim() }, manual?.step);
          store.showWorkflows("run", picked.workflow.id, run.id);
        } catch (e) {
          toast.err(errorText(e));
        }
      })();
      return;
    }
    if (agentSlug) {
      void store.startAgent({
        slug: agentSlug,
        team: team?.team.slug,
        prompt: prompt.trim(),
        model: model || undefined,
        conversation: continuing?.id,
        memory: { conversation: useConversation, long_term: useMemory, knowledge: useKnowledge, operations: useMemory },
        attachments: attached.paths,
        ...limits,
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
      attachments: attached.paths,
      ...limits,
    });
  }
</script>

<svelte:window onkeydown={onkey} />

<form class="h-full overflow-y-auto" onsubmit={submit}>
  <div class="mx-auto max-w-[1100px] p-6">
    <PageHeader title={tr("common.newRun")} subtitle={tr("newRun.sayWhatYouWant")} glow />

    <div class="grid items-start gap-5 lg:grid-cols-[minmax(0,1fr)_320px]">
      <div class="min-w-0 space-y-5">
        {#if agents.cards.length || workflowChoices.length}
          <section aria-label={tr("newRun.runAs")} class="space-y-3">
            <div class="flex flex-wrap items-center gap-3">
              <span class="text-sm font-medium">{tr("newRun.runAs")}</span>
              <Segmented
                label={tr("newRun.runAs")}
                bind:value={() => mode, setMode}
                options={[
                  { value: "plain", label: "Plain run", icon: "terminal" },
                  ...(agents.cards.length ? [{ value: "agent", label: "Agent", icon: "bot" }] : []),
                  ...(agents.teams.length ? [{ value: "team", label: "Team", icon: "users" }] : []),
                  ...(workflowChoices.length ? [{ value: "workflow", label: "Workflow", icon: "workflow" }] : []),
                ]}
              />
            </div>
            {#if mode === "team"}
              <ul class="grid gap-2.5 sm:grid-cols-2" aria-label={tr("common.teams")}>
                {#each agents.teams as t (t.team.slug)}
                  {@const on = choice === `team:${t.team.slug}`}
                  <li>
                    <button type="button" aria-pressed={on} onclick={() => (choice = `team:${t.team.slug}`)} class="flex w-full items-center gap-3 rounded-xl border px-3 py-2.5 text-left transition duration-150 {on ? 'border-accent/50 bg-accent-soft' : 'border-line bg-panel hover:border-line-strong'}">
                      <Avatar emoji={t.team.avatar.emoji} color={t.team.avatar.color} size="md" />
                      <span class="min-w-0 flex-1"><span class="block truncate text-md font-semibold">{t.team.name}</span><span class="block truncate text-xs text-muted">{tr("newRun.leadAgents", { value: t.team.members.length + 1 })}</span></span>
                      {#if on}<Icon name="check" size={15} class="text-accent" />{/if}
                    </button>
                  </li>
                {/each}
              </ul>
            {:else if mode === "workflow"}
              <ul class="grid gap-2.5 sm:grid-cols-2" aria-label={tr("common.workflows")}>
                {#each workflowChoices as c (c.workflow.id)}
                  {@const on = choice === `workflow:${c.workflow.id}`}
                  <li>
                    <button type="button" aria-pressed={on} onclick={() => (choice = `workflow:${c.workflow.id}`)} class="flex w-full items-center gap-3 rounded-xl border px-3 py-2.5 text-left transition duration-150 {on ? 'border-accent/50 bg-accent-soft' : 'border-line bg-panel hover:border-line-strong'}">
                      <span class="grid size-8 shrink-0 place-items-center rounded-lg bg-node-trigger/14 text-node-trigger"><Icon name="workflow" size={16} /></span>
                      <span class="min-w-0 flex-1"><span class="block truncate text-md font-semibold">{c.workflow.draft.name}</span><span class="block truncate text-xs text-muted">v{c.workflow.live_version} · {c.workflow.draft.steps.length} steps</span></span>
                      {#if on}<Icon name="check" size={15} class="text-accent" />{/if}
                    </button>
                  </li>
                {/each}
              </ul>
              <p class="text-sm text-muted">{tr("newRun.whatYouWriteBelow")} <span class="mono">trigger.payload.prompt</span>{tr("newRun.itsOwnStepsDecide")}</p>
            {:else if mode === "agent"}
              <ul class="grid gap-2.5 sm:grid-cols-2" aria-label={tr("common.agents")}>
                {#each agents.cards as c (c.slug)}
                  {@const on = choice === c.slug}
                  <li>
                    <button type="button" aria-pressed={on} onclick={() => (choice = c.slug)} class="flex w-full items-center gap-3 rounded-xl border px-3 py-2.5 text-left transition duration-150 {on ? 'border-accent/50 bg-accent-soft' : 'border-line bg-panel hover:border-line-strong'}">
                      <Avatar emoji={c.emoji} color={c.color} size="md" />
                      <span class="min-w-0 flex-1"><span class="block truncate text-md font-semibold">{c.name}</span><span class="mono block truncate text-xs text-muted">v{c.version} · {c.default_model}</span></span>
                      {#if on}<Icon name="check" size={15} class="text-accent" />{/if}
                    </button>
                  </li>
                {/each}
              </ul>
            {/if}
            {#if team}
              <p class="text-sm text-muted">{tr("newRun.leadsAndMayHand", { lead: team.lead?.name ?? team.team.lead, count: team.team.members.length })}</p>
            {:else if agentDetail}
              <p class="text-sm text-muted">{tr("newRun.itKeepsItsInstructions")}</p>
            {/if}
          </section>
        {/if}

        <Card title={tr("newRun.whatShouldItDo")} icon="sparkles">
          <div class="relative">
            <Textarea id="prompt" rows={5} bind:value={prompt} maxlength={20000} placeholder={tr("newRun.forExampleWhatIs")} aria-label={tr("newRun.whatShouldItDo")} />
            {#if mode !== "workflow"}<AtMenu field={promptField} bind:value={prompt} workspace={workspace.trim() || undefined} onchoose={(path) => void attached.add([path])} />{/if}
            {#if attached.dropping && mode !== "workflow"}
              <div class="pointer-events-none absolute inset-0 grid place-items-center rounded-lg border-2 border-dashed border-accent bg-accent-soft text-base font-medium text-accent" role="status">
                <span class="inline-flex items-center gap-2"><Icon name="paperclip" size={16} />{tr("attach.dropFilesToAttach")}</span>
              </div>
            {/if}
          </div>
          {#if mode !== "workflow"}<p class="mt-1.5 text-xs text-muted">{tr("newRun.plainWordsAttachDocuments")}</p>{/if}
          <div class="mt-3 flex flex-wrap gap-2">
            {#each examples as example}<Chip onclick={() => (prompt = example)}>{example}</Chip>{/each}
          </div>
          {#if mode !== "workflow"}
            <div class="mt-4">
              <AttachedList files={attached.files} onremove={(path) => attached.remove(path)} onattach={() => void attached.choose()} />
            </div>
          {/if}
        </Card>

        {#if mode !== "workflow"}
          <Card title={tr("newRun.limits")} icon="gauge">
            <p class="mb-3 text-sm text-muted">{agentSlug ? tr("newRun.limitsOnlyNarrow") : tr("newRun.limitsStopTheRun")}</p>
            <div class="grid gap-4 sm:grid-cols-3">
              <Field label={tr("newRun.budget")} hint={tr("newRun.dollarsEmptyForNone")} error={budgetOk ? null : tr("newRun.anAmountAboveZero")}>
                {#snippet children(f)}<Input id={f.id} inputmode="decimal" icon="dollar" bind:value={budget} invalid={!budgetOk} placeholder="0.60" />{/snippet}
              </Field>
              <Field label={tr("newRun.time")} hint={tr("newRun.minutesOfWork")} error={minutesOk ? null : tr("newRun.oneToMinutes")}>
                {#snippet children(f)}<Input id={f.id} inputmode="numeric" icon="clock" bind:value={minutes} invalid={!minutesOk} placeholder="5" />{/snippet}
              </Field>
              <Field label={tr("newRun.steps")} hint={tr("newRun.modelStepsAtMost")} error={stepsOk ? null : tr("newRun.oneToSteps")}>
                {#snippet children(f)}<Input id={f.id} inputmode="numeric" icon="list" bind:value={steps} invalid={!stepsOk} placeholder="12" />{/snippet}
              </Field>
            </div>
          </Card>
        {/if}

        {#if !agentSlug}
          <Card title={tr("newRun.whatItMayUse")} icon="wrench">
            <p class="mb-3 text-sm text-muted">{tr("newRun.anythingMarked")} <span class="font-semibold text-fg">asks</span> {tr("newRun.waitsForYouBefore")}</p>
            <div class="flex flex-wrap gap-2">
              {#each builtinTools as tool (tool.id)}
                <Chip selected={isOn(tool.id)} onclick={() => (overrides[tool.id] = !isOn(tool.id))}>
                  <span class="mono">{tool.id}</span>{#if tool.requires_approval}<span class="ml-1.5 font-semibold text-muted">asks</span>{/if}
                </Chip>
              {/each}
            </div>
          </Card>

          <Card title={tr("newRun.serversAndServices")} icon="plug">
            {#if !reach}
              <p class="text-sm text-muted">{tr("newRun.nothingIsConnectedYet")} <button type="button" class="text-accent underline" onclick={() => store.showIntegrations("catalogue")}>{tr("newRun.connectAServerOr")}</button> {tr("newRun.toLetARun")}</p>
            {:else}
              <p class="mb-3 text-sm text-muted">{tr("newRun.aRunReachesOnly")}</p>
              {#if readyServers.length}
                <p class="mb-1.5 text-xs font-medium text-muted">{tr("newRun.mcpServers")}</p>
                <div class="mb-4 flex flex-wrap gap-2">
                  {#each readyServers as server (server.name)}
                    <button
                      type="button"
                      aria-pressed={!!pickedServers[server.name]}
                      onclick={() => (pickedServers[server.name] = !pickedServers[server.name])}
                      class="flex items-center gap-2 rounded-xl border px-2.5 py-1.5 text-left transition duration-150 {pickedServers[server.name] ? 'border-accent/50 bg-accent-soft' : 'border-line bg-panel hover:border-line-strong'}"
                    >
                      <BrandLogo name={server.logo ?? (server.kind === "http" ? "globe" : "server")} size={28} />
                      <span><span class="block text-base font-medium">{server.name}</span><span class="tnum block text-xs text-muted">{tr("newRun.toolsReady", { tools_ready: server.tools_ready })}</span></span>
                      {#if pickedServers[server.name]}<Icon name="check" size={14} class="text-accent" />{/if}
                    </button>
                  {/each}
                </div>
              {/if}
              {#if databases.length}
                <p class="mb-1.5 text-xs font-medium text-muted">{tr("newRun.databases")}</p>
                <div class="mb-4 space-y-2">
                  {#each databases as db (db.id)}
                    <div class="flex flex-wrap items-center gap-3 rounded-xl border px-3 py-2 {pickedDbs[db.name] ? 'border-accent/50 bg-accent-soft' : 'border-line bg-panel'}">
                      <Checkbox checked={!!pickedDbs[db.name]} onchange={(e) => (pickedDbs[db.name] = e.currentTarget.checked)}>
                        <BrandLogo name={db.kind} size={24} class="mr-1 inline-grid align-middle" /> <span class="mono text-sm font-medium">{db.name}</span>
                      </Checkbox>
                      {#if pickedDbs[db.name]}<Switch bind:checked={dbWrites[db.name]} class="ml-auto">{tr("newRun.allowWritesEachAsks")}</Switch>{/if}
                    </div>
                  {/each}
                </div>
              {/if}
              <p class="mb-1.5 text-xs font-medium text-muted">{tr("common.code")}</p>
              <div class="mb-4 flex flex-wrap gap-2">
                <Chip selected={!!pickedCode.javascript} onclick={() => (pickedCode.javascript = !pickedCode.javascript)}>{tr("newRun.runJavascript")}</Chip>
                <Chip selected={!!pickedCode.python} onclick={() => (integrations.sandbox?.python ? (pickedCode.python = !pickedCode.python) : store.showIntegrations("connections"))}>
                  {tr("newRun.runPython")}{integrations.sandbox?.python ? "" : tr("newRun.installItFirst")}
                </Chip>
              </div>
              <div class="grid gap-3 sm:grid-cols-2">
                <Field label={tr("newRun.webHosts")} hint={tr("newRun.commaSeparatedForExample")}>
                  {#snippet children(f)}<Input id={f.id} mono bind:value={hosts} placeholder="none" />{/snippet}
                </Field>
                <Field label={tr("newRun.repositories")} hint={tr("newRun.ownerRepoOrOwner")}>
                  {#snippet children(f)}<Input id={f.id} mono bind:value={repos} placeholder="none" />{/snippet}
                </Field>
                <Field label={tr("newRun.jiraProjects")} hint={tr("newRun.keyOrKeyWrite")}>
                  {#snippet children(f)}<Input id={f.id} mono bind:value={jira} placeholder="none" />{/snippet}
                </Field>
                <Field label={tr("newRun.chatChannels")} hint={tr("newRun.slackAlertsDiscordChannel")}>
                  {#snippet children(f)}<Input id={f.id} mono bind:value={chat} placeholder="none" />{/snippet}
                </Field>
              </div>
            {/if}
          </Card>

          <Card title={tr("newRun.workspaceFolder")} icon="folder">
            <p class="mb-3 text-sm text-muted">{tr("newRun.theFileToolsAre")}</p>
            <div class="flex gap-2">
              <Input class="min-w-0 flex-1" mono bind:value={workspace} placeholder={tr("common.noFolderChosen")} aria-label={tr("newRun.workspaceFolder")} />
              <Button icon="folder" onclick={pickWorkspace}>{tr("common.choose")}</Button>
            </div>
            {#if pickError}<p class="mt-2 text-sm text-err" role="alert">{pickError}</p>{/if}
            {#if workspace}
              <div class="mt-4 grid grid-cols-2 gap-3">
                <Field label={tr("newRun.mayReadPatterns")}>
                  {#snippet children(f)}<Input id={f.id} mono bind:value={allowRead} placeholder={tr("newRun.orDocs")} />{/snippet}
                </Field>
                <Field label={tr("newRun.mayWritePatterns")}>
                  {#snippet children(f)}<Input id={f.id} mono bind:value={allowWrite} placeholder={tr("newRun.noneOrOut")} />{/snippet}
                </Field>
              </div>
            {/if}
          </Card>
        {/if}

        <Card title={tr("newRun.modelAndMemory")} icon="cpu">
          <Field label={tr("newRun.whichModelAnswers")}>
            {#snippet children(f)}
              <Select id={f.id} bind:value={model}>
                <option value="">{agentDetail ? tr("newRun.agentDefault") : tr("newRun.default")} ({agentDetail ? agentDetail.version.spec.models.default : defaultName})</option>
                <option value="demo">{tr("newRun.demoBuiltInNo")}</option>
                {#each store.models?.routes ?? [] as r}<option value={"route:" + r}>route: {r}</option>{/each}
                {#each store.models?.aliases ?? [] as a}<option value={a}>alias: {a}</option>{/each}
                {#each store.models?.models ?? [] as m (m.reference)}
                  <option value={m.reference}>{m.reference}{m.local ? tr("newRun.onThisComputer") : ""}</option>
                {/each}
              </Select>
            {/snippet}
          </Field>
          <div class="mt-3">
            {#if where.kind === "cloud"}
              <Callout tone="info" icon="cloud">{tr("newRun.receivesTheConversationOf", { value: providerName(where.provider) })}</Callout>
            {:else if where.kind === "routed"}
              <Callout tone="info" icon="route">{tr("newRun.theRoute")} <span class="mono">{where.name}</span> {tr("newRun.decidesWhichProviderReceives")}</Callout>
            {:else if where.kind === "local"}
              <Badge tone="ok" dot>{tr("newRun.staysOnThisComputer")}</Badge>
            {:else}
              <p class="flex items-center gap-1.5 text-sm text-muted"><Icon name="info" size={14} /> {tr("newRun.noProviderIsSet")}</p>
            {/if}
          </div>

          <div class="mt-5 border-t border-line-soft pt-4">
            {#if continuing}
              <Callout tone="info" icon="history">
                {tr("newRun.continuingAConversationThat", { prompt: continuing.prompt.length > 70 ? continuing.prompt.slice(0, 70) + "…" : continuing.prompt })}
                <button type="button" class="ml-1 underline" onclick={() => (store.continuing = null)}>{tr("newRun.startANewOne")}</button>
              </Callout>
            {:else}
              <p class="text-sm text-muted">{tr("newRun.thisStartsANew")}</p>
            {/if}
            <div class="mt-3 space-y-2.5">
              <Switch bind:checked={useConversation}>{tr("newRun.earlierRunsOfThis")}</Switch>
              <Switch bind:checked={useMemory}>{tr("newRun.thingsYouAskedIt")}</Switch>
              <Switch bind:checked={useKnowledge}>{tr("newRun.indexedDocuments")}</Switch>
            </div>
            <p class="mt-3 text-xs text-muted">{tr("newRun.whatIsUsedIs")}</p>
          </div>
        </Card>

        {#if store.error}<Callout tone="err">{store.error}</Callout>{/if}

        <div class="sticky bottom-0 z-20 -mx-6 flex items-center justify-end gap-3 border-t border-line bg-bg/90 px-6 py-3 backdrop-blur">
          <span class="flex items-center gap-1.5 text-xs text-muted"><Kbd>{shortcut}</Kbd> {tr("newRun.toStart")}</span>
          {#if agentSlug && prompt.trim()}<Button type="button" variant="ghost" size="lg" icon="workflow" onclick={() => { workflowName = shorten(prompt.trim(), 40); savingAs = true; }}>{tr("newRun.saveAsWorkflow")}</Button>{/if}
          <Button variant="primary" size="lg" icon="play" type="submit" loading={store.busy} disabled={!prompt.trim() || !limitsOk || (mode === "workflow" && !picked)}>{mode === "workflow" ? tr("newRun.runWorkflow") : tr("newRun.startRun")}</Button>
        </div>
      </div>

      <aside class="space-y-5 lg:sticky lg:top-0" aria-label={tr("newRun.beforeYouStart")}>
        <Card title={tr("newRun.beforeYouStart")} icon="shield">
          <KeyValue
            items={[
              { key: team ? "Lead" : agentSlug ? "Agent" : "Run as", value: team ? (team.lead?.name ?? team.team.lead) : agentDetail ? `${agentDetail.version.spec.name} v${agentDetail.agent.current_version}` : "Plain run" },
              { key: "May ask", value: team ? team.members.map((m) => m.name).join(", ") : null },
              { key: "Tools", value: `${grants.granted} granted${grants.asks ? `, ${grants.asks} ask first` : ""}` },
              { key: "Model", value: where.kind === "demo" ? "demo" : model || (agentDetail ? agentDetail.version.spec.models.default : defaultName), mono: true },
              { key: "Memory", value: memoryOn },
              { key: tr("newRun.attached"), value: attached.files.length ? tr("newRun.countFilesSize", { count: attached.files.length, size: fmt.bytes(attached.bytes) }) : null },
              { key: tr("newRun.expected"), value: expected },
              { key: tr("newRun.budget"), value: limits.max_cost_micros ? fmt.usd2(limits.max_cost_micros) : null },
            ]}
          />
          {#if expectedBasis}<p class="mt-2 text-xs text-faint">{expectedBasis} {tr("newRun.itIsAGuess")}</p>{/if}
          <p class="mt-3 text-xs text-muted">{tr("newRun.anythingThatChangesSomething")}</p>
        </Card>
        {#if recent.length}
          <Card title={team ? tr("newRun.recentWithThisTeam") : tr("newRun.recentWithThisAgent")} icon="history" padding="none">
            <ul class="divide-y divide-line-soft">
              {#each recent as run (run.id)}
                <li>
                  <button type="button" class="flex w-full items-center gap-2 px-4 py-2.5 text-left transition duration-150 hover:bg-hover" onclick={() => void store.select(run.id)}>
                    <span class="min-w-0 flex-1"><span class="block truncate text-base">{shorten(run.request.prompt, 48)}</span><span class="text-xs text-muted">{ago(run.created_at, store.now)}</span></span>
                    <StatusBadge state={run.state} />
                  </button>
                </li>
              {/each}
            </ul>
          </Card>
        {/if}
      </aside>
    </div>
  </div>
</form>

<Modal bind:open={savingAs} title={tr("newRun.saveAsWorkflow")} icon="workflow" size="sm">
  <p class="text-base text-muted">{tr("newRun.thisMakesADraft")} {team ? tr("newRun.theTeam") : tr("newRun.theAgent")} {tr("newRun.whatYouWroteYou")}</p>
  <form
    class="mt-4"
    onsubmit={(e) => {
      e.preventDefault();
      void saveAsWorkflow();
    }}
  >
    <Field label={tr("common.name")} hint={slugify(workflowName) ? tr("newRun.shortName", { value: slugify(workflowName) }) : tr("newRun.useSomeLettersOr")}>
      {#snippet children(f)}<Input id={f.id} bind:value={workflowName} maxlength={80} />{/snippet}
    </Field>
    <div class="mt-4 flex justify-end gap-2">
      <Button type="button" onclick={() => (savingAs = false)}>{tr("common.cancel")}</Button>
      <Button type="submit" variant="primary" disabled={savingBusy || !slugify(workflowName)}>{tr("common.save")}</Button>
    </div>
  </form>
</Modal>
