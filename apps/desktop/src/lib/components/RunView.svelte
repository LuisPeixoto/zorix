<script lang="ts">
  import { fmt, t } from "../i18n/index.ts";
  import { ago, duration, isFinished, isLive, shortId, tokens } from "../format.ts";
  import { providerCount, runSteps } from "../steps.ts";
  import { usd } from "../shell.ts";
  import { store } from "../stores/runs.svelte.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import { Badge, Button, Callout, Card, Icon, IconButton, KeyValue, List, ListRow, Markdown, Sparkbar, Stat, Tabs, ToolCallCard, TreeView } from "../ui/index.ts";
  import { type Execution, type TreeApproval, runTree, treeApprovals } from "../api.ts";
  import { agents } from "../stores/agents.svelte.ts";
  import { buildTree, totals } from "../tree.ts";
  import TreeApprovals from "./agents/TreeApprovals.svelte";
  import ApprovalCard from "./ApprovalCard.svelte";
  import ServerAskCard from "./ServerAskCard.svelte";
  import { asks } from "../stores/asks.svelte.ts";
  import StatusBadge from "./StatusBadge.svelte";
  import Timeline from "./Timeline.svelte";

  const detail = $derived(store.detail);
  const run = $derived(detail?.execution);
  const pending = $derived(detail?.approvals.filter((a) => a.status === "pending") ?? []);
  const modelSteps = $derived(detail?.summary.steps.filter((s) => s.kind === "model") ?? []);

  // The context is read again when the run changes state: the recall is made when the first model step starts.
  const runId = $derived(run?.id);
  const runState = $derived(run?.state);
  $effect(() => {
    if (runId && runState) void store.loadContext(runId);
  });
  // The tree this run belongs to: what it started, or what started it. Read again while anything in it is still working.
  let family = $state<Execution[]>([]);
  let waitingBelow = $state<TreeApproval[]>([]);
  let familyTimer: ReturnType<typeof setTimeout> | null = null;
  const topId = $derived(run?.root ?? run?.id);
  async function loadFamily() {
    const id = topId;
    if (!id) return;
    try {
      const [all, waiting] = await Promise.all([runTree(id), treeApprovals(id)]);
      if (topId === id) {
        family = all;
        waitingBelow = waiting.filter((w) => w.execution !== run?.id);
      }
    } catch {
      /* The tree is a nicety: the run itself is shown either way. */
    }
    if (familyTimer) clearTimeout(familyTimer);
    if (topId === id && family.some((e) => !["completed", "failed", "cancelled"].includes(e.state))) familyTimer = setTimeout(loadFamily, 1500);
  }
  $effect(() => {
    void topId;
    void runState;
    family = [];
    waitingBelow = [];
    void loadFamily();
    return () => {
      if (familyTimer) clearTimeout(familyTimer);
    };
  });
  const tree = $derived(family.length > 1 ? buildTree(family) : null);
  // A server may ask during a tool call of this run or of one below it.
  const asked = $derived(asks.list.filter((a) => a.execution_id === run?.id || family.some((e) => e.id === a.execution_id)));
  const sums = $derived(tree ? totals(tree) : null);
  const look = (e: Execution) => {
    const card = e.agent ? agents.card(e.agent.slug) : undefined;
    return { emoji: card?.emoji ?? "🤖", color: card?.color ?? "muted", name: card?.name ?? e.agent?.slug ?? t("common.run") };
  };

  const context = $derived(store.context);
  const kindLabel: Record<string, string> = { fact: "remembered", exchange: t("runView.earlierInThisConversation"), chunk: "document", past_work: t("runView.pastWork") };
  const used = $derived(context?.recall?.items ?? []);

  async function copyId(id: string) {
    try {
      await navigator.clipboard.writeText(id);
      toast.ok(t("runView.runIdCopied"));
    } catch {
      toast.err(t("runView.couldNotCopyThe"));
    }
  }

  const steps = $derived(detail ? runSteps(detail.summary.steps, detail.tool_calls) : []);
  const approvedByYou = $derived(detail?.approvals.filter((a) => a.status === "approved").length ?? 0);
  const team = $derived(run?.agent?.team ?? null);
  const helpers = $derived(Math.max(0, family.length - 1));

  // The tab shown. A run with sub-runs opens on its Work tree, any other on its Steps.
  let chosen = $state<string | null>(null);
  const tab = $derived(chosen ?? (tree ? "tree" : "steps"));
  $effect(() => {
    void runId;
    chosen = null;
  });
  const tabs = $derived([
    ...(tree ? [{ value: "tree", label: t("runView.workTree"), count: sums?.runs }] : []),
    { value: "steps", label: t("runView.steps"), count: steps.length },
    { value: "events", label: t("runView.events"), count: store.events.length },
    { value: "context", label: t("runView.context") },
    { value: "cost", label: t("runView.cost") },
  ]);
  const resultText = (value: unknown) => (value === undefined ? "" : typeof value === "string" ? value : JSON.stringify(value, null, 2));
</script>

{#if run && detail}
  <div class="h-full overflow-y-auto">
    <div class="mx-auto w-full max-w-[1200px] p-6">
      <header class="relative mb-5">
        <span class="glow -top-20 -left-16 size-56" aria-hidden="true"></span>
        <nav aria-label={t("runView.whereYouAre")} class="relative flex flex-wrap items-center gap-2 text-sm text-muted">
          <button type="button" class="inline-flex items-center gap-1 transition hover:text-fg" onclick={() => store.showRuns()}><Icon name="history" size={13} />{t("common.runs")}</button>
          <Icon name="chevron-right" size={12} class="text-faint" />
          <span class="mono selectable" title={run.id}>{shortId(run.id)}</span>
          <IconButton label={t("runView.copyTheRunId")} icon="copy" size="sm" onclick={() => copyId(run.id)} />
          <StatusBadge state={run.state} />
          {#if run.parent}
            <button type="button" class="inline-flex h-5 items-center gap-1.5 rounded-full bg-hover px-2 text-xs font-medium text-muted transition hover:text-fg" title={t("runView.openTheRunThat")} onclick={() => void store.select(run.root ?? run.parent!)}>
              <Icon name="branch" size={12} /> {t("runView.subRunLevel", { value: run.depth ?? 1 })}
            </button>
          {/if}
          {#if run.agent}
            <button type="button" class="inline-flex h-5 items-center gap-1.5 rounded-full bg-accent-soft px-2 text-xs font-medium text-accent transition hover:bg-select" title={t("runView.openAndThisConversation", { slug: run.agent.slug })} onclick={() => store.showAgents(run.agent!.slug, "chat", run.conversation ?? null, team)}>
              <Icon name={team ? "users" : "bot"} size={12} /> {team ?? run.agent.slug} · v{run.agent.version}
            </button>
          {/if}
        </nav>
        <div class="relative mt-2 flex flex-wrap items-end justify-between gap-4">
          <div class="min-w-0 flex-1">
            <h1 class="selectable text-3xl leading-tight font-semibold tracking-tight break-words text-balance">{run.request.prompt}</h1>
            <p class="mt-1.5 text-base text-muted">
              {t("runView.started", { value: ago(run.created_at, store.now) })}{#if helpers > 0}{" · "}{team ?? run.agent?.slug ?? t("runView.thisRun")} asked {helpers} {helpers === 1 ? "agent" : "agents"}{/if}
              {#if run.request.metadata?.task}
                {t("runView.fromTask", { value: " · " })}
                <button type="button" class="mono rounded font-medium text-accent outline-none hover:underline focus-visible:underline" onclick={() => store.showProjects(run.request.metadata?.project ?? "", "board", run.request.metadata?.task ?? null)}>{run.request.metadata.task}</button>
              {:else if run.request.metadata?.project_manager}
                {t("runView.planning", { value: " · " })}
                <button type="button" class="mono rounded font-medium text-accent outline-none hover:underline focus-visible:underline" onclick={() => store.showProjects(run.request.metadata?.project ?? "")}>{run.request.metadata.project}</button>
              {/if}
              {#if run.request.metadata?.workflow_run}
                {t("runView.fromWorkflow", { value: " · " })}
                <button type="button" class="rounded font-medium text-accent outline-none hover:underline focus-visible:underline" onclick={() => store.showWorkflows("run", null, run.request.metadata?.workflow_run ?? null)}>{run.request.metadata.workflow ?? t("runView.aWorkflow")}</button>
              {/if}
            </p>
          </div>
          <div class="flex shrink-0 items-center gap-2">
            {#if run.state === "paused"}<Button variant="primary" icon="play" disabled={store.busy} onclick={() => store.resume()}>{t("runView.resume")}</Button>{/if}
            {#if isLive(run.state)}<Button icon="pause" disabled={store.busy} onclick={() => store.pause()}>{t("runView.pause")}</Button>{/if}
            {#if !isFinished(run.state)}<Button variant="danger" icon="x" disabled={store.busy} onclick={() => store.cancel()}>{t("common.cancel")}</Button>{/if}
            {#if run.conversation}
              <Button variant="ghost" icon="message" onclick={() => (run.agent ? store.showAgents(run.agent.slug, "chat", run.conversation ?? null, team) : store.continueConversation())}>{t("runView.continueInChat")}</Button>
            {/if}
          </div>
        </div>
      </header>

      <div class="mb-5 grid grid-cols-2 gap-4 lg:grid-cols-4">
        <Stat icon="cpu" label={t("runView.modelSteps")} value={detail.summary.model_calls} detail="{providerCount(detail.summary.steps)} {providerCount(detail.summary.steps) === 1 ? 'provider' : 'providers'}" />
        <Stat icon="wrench" label={t("runView.toolCalls")} value={run.usage.tool_calls} detail="{approvedByYou} approved by you" />
        <Stat icon="sparkles" label={t("runView.tokens")} value={fmt.number(tokens(run.usage))} detail="{run.usage.input_tokens.toLocaleString('fr-FR')} in · {run.usage.output_tokens.toLocaleString('fr-FR')} out" />
        <Stat icon="clock" label={t("runView.workTime")} value={duration(run.active_ms)} detail="{usd(run.usage.cost_micros)} · waiting for you is not counted" />
      </div>

      <div class="grid items-start gap-5 lg:grid-cols-[minmax(0,1fr)_360px]">
        <div class="min-w-0 space-y-5">
          {#if run.result}
            <Card title={t("runView.result")} icon="check-circle">
              {#if run.agent}<Markdown text={run.result.text} />{:else}<pre class="selectable font-sans text-md leading-relaxed whitespace-pre-wrap">{run.result.text}</pre>{/if}
            </Card>
          {/if}
          {#if run.failure}
            <Callout tone="err" title={t("runView.itDidNotFinish")}><span class="selectable">{run.failure.message}</span></Callout>
          {/if}
          {#if run.state === "cancelled"}
            <Callout tone="muted" icon="stop" title={t("common.cancelled")}>{run.cancel_reason ?? ""}</Callout>
          {/if}

          <Tabs {tabs} bind:value={() => tab, (v) => (chosen = v)} label={t("runView.runDetails")} />

          {#if tab === "tree" && tree && sums}
            <div class="space-y-3">
              <TreeView root={tree} selected={run.id} {look} now={store.now} onselect={(id) => id !== run.id && void store.select(id)} />
              <p class="text-xs text-muted tabular-nums">{sums.runs} {t("runView.runs2")} {sums.done} done{sums.working ? ` · ${sums.working} working` : ""}{sums.waiting ? t("runView.waitingForYou", { waiting: sums.waiting }) : ""}{sums.failed ? ` · ${sums.failed} failed` : ""}{t("runView.theCostAndTokens")}</p>
            </div>
          {:else if tab === "steps"}
            <section aria-label={t("runView.steps")} class="space-y-2.5">
              <h2 class="flex items-center gap-2 text-base font-semibold"><Icon name="list" size={14} class="text-muted" />{t("runView.stepsOf")} {team ?? run.agent?.slug ?? t("runView.thisRun")}</h2>
              {#each steps as item, i (i)}
                {#if item.kind === "tool" && item.call}
                  <ToolCallCard tool={item.title} status={item.call.status === "delegated" ? "started" : item.call.status} args={item.call.arguments} result={resultText(item.call.result)} error={item.call.error} />
                {:else}
                  <div class="flex items-center gap-2.5 rounded-xl border border-line bg-panel-2 px-3 py-2">
                    <span class="grid size-6 shrink-0 place-items-center rounded-md bg-panel text-muted shadow-1"><Icon name={item.kind === "model" ? "cpu" : "wrench"} size={13} /></span>
                    <span class="mono min-w-0 flex-1 truncate text-sm font-medium">{item.title}</span>
                    <span class="mono text-xs text-muted tabular-nums">{duration(item.ms)}</span>
                  </div>
                {/if}
              {:else}
                <p class="text-base text-muted">{t("runView.noStepsYet")}</p>
              {/each}
              <p class="flex items-center gap-1.5 pt-1 text-xs {detail.summary.fallbacks > 0 ? 'text-warn' : 'text-muted'}">
                <Icon name={detail.summary.fallbacks > 0 ? "route" : "check"} size={13} />
                {detail.summary.fallbacks > 0 ? t("runView.switchedModelTimeS", { fallbacks: detail.summary.fallbacks }) : t("runView.noFallbackWasNeeded")}
              </p>
            </section>
          {:else if tab === "events"}
            <Card title={t("runView.whatHappened")} icon="history"><Timeline events={store.events} /></Card>
          {:else if tab === "context"}
            {#if context?.recall || context?.segments}
              <Card title={t("runView.context")} icon="book">
                {#if context.recall?.degraded}<Callout tone="warn" class="mb-3">{t("runView.searchWasWeakerThan", { degraded: context.recall.degraded })}</Callout>{/if}
                {#if context.recall?.scrubbed}<p class="mb-3 text-xs text-muted">{t("runView.somethingThisRunUsed")}</p>{/if}
                {#if used.length}
                  <ul class="space-y-2">
                    {#each used as item (item.id + item.kind)}
                      <li class="rounded-lg border border-line bg-panel p-3">
                        <div class="flex flex-wrap items-center gap-1.5">
                          <Badge tone={item.kind === "fact" ? "accent" : item.kind === "chunk" ? "info" : "muted"}>{kindLabel[item.kind]}</Badge>
                          <span class="selectable mono truncate text-xs text-muted">{item.label}</span>
                          {#each item.found_by as by}<Badge tone="muted">{by}</Badge>{/each}
                          <span class="ml-auto text-xs text-muted tabular-nums">{item.tokens} tokens</span>
                        </div>
                        <p class="selectable mt-1.5 line-clamp-3 text-sm text-fg/90">{item.text}</p>
                      </li>
                    {/each}
                  </ul>
                {:else}
                  <p class="text-sm text-muted">{t("runView.memoryAddedNothingTo")}</p>
                {/if}
                {#if context.segments?.length}
                  <p class="mt-4 mb-1.5 text-xs font-medium text-muted">{t("runView.whatTheFirstRequest")}{context.token_factor && context.token_factor !== 1 ? t("runView.tokensLearnedFromThis", { value: context.token_factor.toFixed(2) }) : ""}</p>
                  <div class="flex flex-wrap gap-1.5">
                    {#each context.segments as segment, i (i)}<Badge tone={segment.kept ? "muted" : "warn"}>{segment.origin} · {segment.tokens}{segment.kept ? "" : t("runView.leftOut")}</Badge>{/each}
                  </div>
                {/if}
              </Card>
            {:else}
              <p class="text-base text-muted">{t("runView.theContextIsBuilt")}</p>
            {/if}
          {:else}
            <Card title={t("runView.cost")} icon="dollar">
              <KeyValue
                items={[
                  { key: "Cost", value: usd(run.usage.cost_micros), mono: true },
                  { key: "Input tokens", value: fmt.number(run.usage.input_tokens), mono: true },
                  { key: "Output tokens", value: fmt.number(run.usage.output_tokens), mono: true },
                  { key: "Model calls", value: run.usage.model_calls, mono: true },
                  { key: "Tool calls", value: run.usage.tool_calls, mono: true },
                ]}
              />
              {#if tree && tree.children.length}
                <p class="mt-4 mb-2 text-2xs font-semibold tracking-wider text-faint uppercase">{t("runView.whereItWent")}</p>
                <ul class="space-y-2">
                  {#each family as member (member.id)}
                    <li class="flex items-center gap-3 text-sm">
                      <span class="min-w-0 flex-1 truncate">{look(member).name}{member.id === run.id ? "" : ` · ${member.request.prompt.slice(0, 40)}`}</span>
                      <Sparkbar value={member.usage.cost_micros} max={run.root ? (family.find((f) => f.id === run.root)?.usage.cost_micros ?? 1) : run.usage.cost_micros} label={t("common.shareOfTheCost", { name: look(member).name })} />
                      <span class="mono w-16 text-right text-xs text-muted tabular-nums">{usd(member.usage.cost_micros)}</span>
                    </li>
                  {/each}
                </ul>
              {/if}
            </Card>
          {/if}
        </div>

        <aside class="min-w-0 space-y-5" aria-label={t("runView.thisRun2")}>
          {#each asked as ask (ask.id)}<ServerAskCard {ask} />{/each}
          {#each pending as approval (approval.id)}<ApprovalCard {approval} />{/each}
          <TreeApprovals items={waitingBelow} onanswered={() => { void loadFamily(); void store.select(run.id); }} />
          <Card title={t("runView.thisRun2")} icon="shield">
            <KeyValue
              items={[
                { key: "Started by", value: run.agent ? `${team ?? run.agent.slug} v${run.agent.version}` : "you" },
                { key: "Model", value: run.request.model, mono: true },
                { key: "Steps", value: `${run.iteration} of the loop`, mono: false },
                { key: "Tools asked for", value: run.tool_calls_requested, mono: true },
                { key: "Cost", value: usd(run.usage.cost_micros), mono: true },
              ]}
            />
            {#if tree}<p class="mt-3 text-xs text-muted">{t("runView.budgetsAndPermissionsOnly")}</p>{/if}
          </Card>
        </aside>
      </div>
    </div>
  </div>
{:else}
  <div class="mx-auto max-w-4xl space-y-4 p-6" aria-busy="true" aria-label={t("common.loading")}>
    <div class="skeleton h-6 w-24"></div>
    <div class="skeleton h-8 w-2/3"></div>
    <div class="grid grid-cols-4 gap-3"><div class="skeleton h-20"></div><div class="skeleton h-20"></div><div class="skeleton h-20"></div><div class="skeleton h-20"></div></div>
  </div>
{/if}
