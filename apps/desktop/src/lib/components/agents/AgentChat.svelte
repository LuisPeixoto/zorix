<script lang="ts">
  import { onDestroy, tick, untrack } from "svelte";

  import {
    type AgentConversation,
    type AgentDetail,
    type Execution,
    type StoredCase,
    type ThreadRun,
    type TreeApproval,
    agentAsk,
    agentThreads,
    decideRun,
    errorText,
    evalCases,
    openExternal,
    runTree,
    stopLive,
    threadRuns,
    treeApprovals,
    watchLive,
  } from "../../api.ts";
  import { ago, shorten } from "../../format.ts";
  import { agents } from "../../stores/agents.svelte.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { DELEGATE, childRows } from "../../delegation.ts";
  import { buildThread, canSend, openRun } from "../../thread.ts";
  import { Avatar, Badge, Button, ChatBubble, Chip, Composer, DelegationCard, Icon, Markdown, ToolCallCard } from "../../ui/index.ts";
  import TreeApprovals from "./TreeApprovals.svelte";
  import type { ToolStatus } from "../../ui/ToolCallCard.svelte";

  import { store as runs_store } from "../../stores/runs.svelte.ts";

  let { detail }: { detail: AgentDetail } = $props();
  const slug = $derived(detail.agent.slug);
  const spec = $derived(detail.version.spec);

  let threads = $state<AgentConversation[]>([]);
  let active = $state<string | null>(null);
  let runs = $state<ThreadRun[]>([]);
  let live = $state<{ runId: string; text: string } | null>(null);
  let draft = $state("");
  let sending = $state(false);
  let suggestions = $state<StoredCase[]>([]);
  let scroller: HTMLDivElement | undefined = $state();
  let stick = true;
  let pollTimer: ReturnType<typeof setTimeout> | null = null;
  let watching: string | null = null;
  let alive = true;
  // Sub-runs: the tree under each run of this chat that delegated, what each one is writing, and what waits for a person.
  let family = $state<Execution[]>([]);
  let asked = $state<TreeApproval[]>([]);
  let childLive = $state<Record<string, string>>({});
  const childWatch = new Set<string>();
  const needsYou = $derived(new Set(asked.map((a) => a.execution)));
  const team = $derived(agents.chatTeam ? agents.team(agents.chatTeam) : undefined);

  const items = $derived(buildThread(runs, live));
  const ready = $derived(canSend(runs));
  const working = $derived(openRun(runs));

  async function loadThreads() {
    try {
      threads = await agentThreads(slug);
    } catch (e) {
      toast.err(errorText(e));
    }
  }

  async function loadRuns() {
    if (!active) {
      runs = [];
      return;
    }
    const conversation = active;
    try {
      const next = await threadRuns(slug, conversation);
      if (alive && active === conversation) {
        runs = next;
        void loadFamily(next);
      }
    } catch (e) {
      toast.err(errorText(e));
    }
  }

  const settledState = (e: Execution) => ["completed", "failed", "cancelled"].includes(e.state);
  const face = (e: Execution) => {
    const card = e.agent ? agents.card(e.agent.slug) : undefined;
    return { name: card?.name ?? e.agent?.slug ?? "Agent", emoji: card?.emoji ?? "🤖", color: card?.color ?? "muted" };
  };

  /** Read the sub-runs of every run here that delegated, and follow the text of those that are writing. */
  async function loadFamily(from: ThreadRun[]) {
    const owners = from.filter((r) => r.tool_calls.some((c) => c.tool === DELEGATE)).map((r) => r.execution.id);
    if (owners.length === 0) {
      family = [];
      asked = [];
      return;
    }
    try {
      const trees = await Promise.all(owners.map((id) => runTree(id)));
      const open = from.filter((r) => owners.includes(r.execution.id) && !settledState(r.execution)).map((r) => r.execution.id);
      const waiting = (await Promise.all(open.map((id) => treeApprovals(id)))).flat();
      if (!alive) return;
      family = trees.flat();
      asked = waiting;
      for (const e of family) {
        if (e.depth && !settledState(e) && !childWatch.has(e.id)) followChild(e.id);
      }
    } catch {
      /* The card then shows what the run itself stored. */
    }
  }

  function followChild(id: string) {
    childWatch.add(id);
    void watchLive(id, (batch) => {
      childLive[id] = batch.reset ? batch.text : (childLive[id] ?? "") + batch.text;
    }).catch(() => {});
  }

  function schedule() {
    if (pollTimer) clearTimeout(pollTimer);
    pollTimer = setTimeout(async () => {
      await loadRuns();
      if (alive && !canSend(runs)) schedule();
      else void loadThreads();
    }, 700);
  }

  // Follow the text of the run that is working, from the moment it starts until it is stored.
  $effect(() => {
    const id = working?.execution.id ?? null;
    untrack(() => {
      if (watching && watching !== id) {
        void stopLive(watching).catch(() => {});
        watching = null;
        live = null;
      }
      if (id && watching !== id) {
        watching = id;
        live = { runId: id, text: "" };
        void watchLive(id, (batch) => {
          if (watching !== id) return;
          live = { runId: id, text: batch.reset ? batch.text : (live?.text ?? "") + batch.text };
        }).catch(() => {});
      }
    });
  });

  $effect(() => {
    if (!ready) untrack(schedule);
  });

  async function select(conversation: string | null) {
    active = conversation;
    runs = [];
    live = null;
    stick = true;
    await loadRuns();
  }

  async function send(text: string) {
    sending = true;
    try {
      const run = await agentAsk({ slug, prompt: text, conversation: active ?? undefined, team: agents.chatTeam ?? undefined });
      active = run.conversation ?? active;
      stick = true;
      await loadRuns();
      void loadThreads();
    } catch (e) {
      draft = text;
      toast.err(errorText(e));
    } finally {
      sending = false;
    }
  }

  const answered = () => void loadRuns();

  async function decide(runId: string, approvalId: string, hash: string, yes: boolean) {
    try {
      await decideRun(runId, approvalId, yes, hash);
      await loadRuns();
    } catch (e) {
      toast.err(errorText(e));
    }
  }

  const toolStatus = (status: string): ToolStatus => status as ToolStatus;

  $effect(() => {
    void items.length;
    void live?.text;
    if (stick) void tick().then(() => scroller?.scrollTo({ top: scroller.scrollHeight }));
  });

  function onscroll() {
    if (!scroller) return;
    stick = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 80;
  }

  // Start from the conversation a caller asked for, otherwise a new one.
  void (async () => {
    await loadThreads();
    const wanted = agents.startConversation;
    agents.startConversation = null;
    await select(wanted ?? null);
    try {
      suggestions = (await evalCases(slug)).slice(0, 3);
    } catch {
      suggestions = [];
    }
  })();

  onDestroy(() => {
    alive = false;
    if (pollTimer) clearTimeout(pollTimer);
    if (watching) void stopLive(watching).catch(() => {});
    for (const id of childWatch) void stopLive(id).catch(() => {});
  });
</script>

<div class="flex h-full min-h-0">
  <aside class="flex w-64 shrink-0 flex-col border-r border-line bg-panel-2" aria-label="Conversations">
    <div class="p-3">
      <Button class="w-full" icon="plus" variant="secondary" onclick={() => select(null)}>New chat</Button>
    </div>
    <nav class="min-h-0 flex-1 overflow-y-auto px-2 pb-2">
      {#each threads as t (t.conversation)}
        <button
          type="button"
          class="mb-0.5 block w-full rounded-lg px-2.5 py-2 text-left transition duration-150 hover:bg-hover {active === t.conversation ? 'bg-accent-soft' : ''}"
          aria-current={active === t.conversation ? "true" : undefined}
          onclick={() => select(t.conversation)}
        >
          <span class="block truncate text-md font-medium">{shorten(t.title, 48)}</span>
          <span class="mt-0.5 flex justify-between text-xs text-muted"><span>{t.runs} message{t.runs === 1 ? "" : "s"}</span><span class="tnum">{ago(t.last_at)}</span></span>
        </button>
      {:else}
        <p class="px-2.5 py-2 text-sm text-muted">No conversations yet.</p>
      {/each}
    </nav>
  </aside>

  <section class="flex min-w-0 flex-1 flex-col">
    <div bind:this={scroller} {onscroll} class="min-h-0 flex-1 overflow-y-auto" role="log" aria-live="polite" aria-label="Conversation with {spec.name}">
      <div class="mx-auto flex max-w-3xl flex-col gap-5 px-6 py-6">
        {#if items.length === 0}
          <div class="mx-auto flex max-w-md flex-col items-center py-16 text-center">
            <Avatar emoji={spec.avatar.emoji} color={spec.avatar.color} size="xl" />
            <h2 class="mt-4 text-2xl font-semibold">{team ? `Ask ${team.team.name}` : `Ask ${spec.name}`}</h2>
            <p class="mt-1.5 text-base text-muted">{team ? `${spec.name} leads and can hand work to ${team.team.members.length} other agent${team.team.members.length === 1 ? "" : "s"}.` : spec.description || "Say what you want done."}</p>
            {#if suggestions.length}
              <div class="mt-5 flex flex-wrap justify-center gap-2" aria-label="Things to try">
                {#each suggestions as s (s.id)}
                  <Chip onclick={() => (draft = s.case.prompt)}>{shorten(s.case.prompt, 54)}</Chip>
                {/each}
              </div>
            {/if}
          </div>
        {/if}

        {#each items as item (item.id)}
          {#if item.kind === "user"}
            <ChatBubble role="user" time={ago(item.at)}><p class="selectable whitespace-pre-wrap">{item.text}</p></ChatBubble>
          {:else if item.kind === "answer"}
            <ChatBubble role="assistant" name={spec.name} time={item.streaming ? "writing…" : `${ago(item.at)}${item.version ? " · v" + item.version : ""}`} streaming={item.streaming}>
              {#snippet avatar()}<Avatar emoji={spec.avatar.emoji} color={spec.avatar.color} />{/snippet}
              {#if item.text}<Markdown text={item.text} onlink={(href) => void openExternal(href).catch((e) => toast.err(errorText(e)))} />{:else}<span class="text-muted">Thinking…</span>{/if}
            </ChatBubble>
          {:else if item.kind === "tool" && item.call.tool === DELEGATE}
            {@const rows = childRows(item.call, family, needsYou, face, childLive)}
            <div class="pl-12">
              {#if rows.length}
                <DelegationCard children={rows} onopen={(id) => void runs_store.select(id)}>
                  {#snippet row(r)}
                    <TreeApprovals items={asked.filter((a) => a.execution === r.id)} compact onanswered={answered} />
                  {/snippet}
                </DelegationCard>
              {:else}
                <ToolCallCard tool={item.call.tool} status={toolStatus(item.call.status === "delegated" ? "started" : item.call.status)} args={item.call.arguments} error={item.call.error} />
              {/if}
            </div>
          {:else if item.kind === "tool"}
            <div class="pl-12">
              {#if item.approval?.status === "pending"}
                <ToolCallCard tool={item.call.tool} status="awaiting_approval" args={item.call.arguments} error={item.call.error} open>
                  <p class="mb-2 flex items-center gap-1.5 text-sm font-medium text-warn"><Icon name="shield" size={14} /> {item.approval.reason || "This step asks for your approval."}</p>
                  <div class="flex gap-2">
                    <Button size="sm" variant="primary" icon="check" onclick={() => decide(item.runId, item.approval!.id, item.approval!.call_hash, true)}>Approve</Button>
                    <Button size="sm" icon="x" onclick={() => decide(item.runId, item.approval!.id, item.approval!.call_hash, false)}>Refuse</Button>
                  </div>
                </ToolCallCard>
              {:else}
                <ToolCallCard tool={item.call.tool} status={toolStatus(item.call.status)} args={item.call.arguments} error={item.call.error} />
              {/if}
            </div>
          {:else if item.kind === "problem"}
            <div class="pl-12"><div class="rounded-xl border px-3.5 py-2.5 text-sm {item.tone === 'err' ? 'border-err/35 bg-err/8 text-err' : 'border-line bg-panel-2 text-muted'}" role={item.tone === "err" ? "alert" : undefined}><strong class="font-semibold">{item.title}.</strong> {item.text}</div></div>
          {:else}
            <p class="pl-12 text-sm text-muted"><Badge tone="warn" dot>Waiting</Badge> <span class="ml-1.5">{item.text}</span></p>
          {/if}
        {/each}
      </div>
    </div>

    <div class="shrink-0 border-t border-line-soft bg-bg/70 px-6 pt-3 pb-4 backdrop-blur">
      <div class="mx-auto max-w-3xl">
        <Composer
          bind:value={draft}
          onsend={send}
          busy={sending || !ready}
          disabled={!!agents.error}
          placeholder={ready ? `Ask ${spec.name}…` : "Waiting for this answer…"}
          label={`Message ${spec.name}`}
        >
          {#snippet tools()}
            {#if team}<Badge tone="info" dot>Team · {team.team.name}</Badge>{/if}
            <Badge tone="muted">{spec.models.default}</Badge>
            <Badge tone="accent">v{detail.agent.current_version}</Badge>
          {/snippet}
        </Composer>
      </div>
    </div>
  </section>
</div>
