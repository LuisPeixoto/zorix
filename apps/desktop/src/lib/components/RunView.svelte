<script lang="ts">
  import { duration, isFinished, isLive, shortId, tokens } from "../format.ts";
  import { store } from "../stores/runs.svelte.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import { Badge, Button, Callout, Card, Icon, IconButton, List, ListRow, Stat } from "../ui/index.ts";
  import ApprovalCard from "./ApprovalCard.svelte";
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
  const context = $derived(store.context);
  const kindLabel: Record<string, string> = { fact: "remembered", exchange: "earlier in this conversation", chunk: "document", past_work: "past work" };
  const used = $derived(context?.recall?.items ?? []);

  async function copyId(id: string) {
    try {
      await navigator.clipboard.writeText(id);
      toast.ok("Run id copied.");
    } catch {
      toast.err("Could not copy the id.");
    }
  }

  const statusLabel = (status: string) => status.replace("_", " ");
  const callTone = (status: string) =>
    status === "succeeded" ? "ok" : status === "failed" ? "err" : status === "denied" ? "warn" : "muted";
</script>

{#if run && detail}
  <div class="flex h-full flex-col overflow-y-auto">
    <header class="relative shrink-0 overflow-hidden border-b border-line bg-panel px-6 py-5">
      <span class="glow -top-24 -right-16 size-60" aria-hidden="true"></span>
      <div class="relative flex items-start justify-between gap-4">
        <div class="min-w-0">
          <StatusBadge state={run.state} />
          <h2 class="selectable mt-2 text-2xl leading-tight font-semibold break-words">{run.request.prompt}</h2>
          <p class="mt-1.5 flex items-center gap-1 text-xs text-faint">
            <span class="mono selectable">{run.id}</span>
            <IconButton label="Copy the run id" icon="copy" size="sm" onclick={() => copyId(run.id)} />
          </p>
        </div>
        <div class="flex shrink-0 gap-2">
          {#if run.state === "paused"}
            <Button variant="primary" icon="play" disabled={store.busy} onclick={() => store.resume()}>Resume</Button>
          {/if}
          {#if isLive(run.state)}
            <Button icon="pause" disabled={store.busy} onclick={() => store.pause()}>Pause</Button>
          {/if}
          {#if isFinished(run.state) && run.conversation}
            <Button icon="history" onclick={() => store.continueConversation()}>Continue</Button>
          {/if}
          {#if !isFinished(run.state)}
            <Button variant="danger" icon="stop" disabled={store.busy} onclick={() => store.cancel()}>Cancel</Button>
          {/if}
        </div>
      </div>

      <dl class="relative mt-5 grid grid-cols-2 gap-3 sm:grid-cols-4">
        <Stat icon="cpu" label="Model steps" value={detail.summary.model_calls} />
        <Stat icon="wrench" label="Tool calls" value={run.usage.tool_calls} />
        <Stat icon="sparkles" label="Tokens" value={tokens(run.usage)} />
        <Stat icon="clock" label="Work time" value={duration(run.active_ms)} detail="Waiting for you is not counted" />
      </dl>
    </header>

    <div class="mx-auto w-full max-w-4xl shrink-0 space-y-5 p-6">
      {#each pending as approval (approval.id)}
        <ApprovalCard {approval} />
      {/each}

      {#if run.result}
        <Card title="Result" icon="check-circle">
          <pre class="selectable font-sans text-md leading-relaxed whitespace-pre-wrap">{run.result.text}</pre>
        </Card>
      {/if}
      {#if run.failure}
        <Callout tone="err" title="It did not finish"><span class="selectable">{run.failure.message}</span></Callout>
      {/if}
      {#if run.state === "cancelled"}
        <Callout tone="muted" icon="stop" title="Cancelled">{run.cancel_reason ?? ""}</Callout>
      {/if}

      {#if context?.recall || context?.segments}
        <Card title="Context" icon="book">
          {#if context.recall?.degraded}
            <Callout tone="warn" class="mb-3">Search was weaker than usual: {context.recall.degraded}</Callout>
          {/if}
          {#if context.recall?.scrubbed}
            <p class="mb-3 text-xs text-faint">Something this run used was deleted afterwards, so it is no longer listed here.</p>
          {/if}
          {#if used.length}
            <ul class="space-y-2">
              {#each used as item (item.id + item.kind)}
                <li class="rounded-lg border border-line bg-panel p-3">
                  <div class="flex flex-wrap items-center gap-1.5">
                    <Badge tone={item.kind === "fact" ? "accent" : item.kind === "chunk" ? "info" : "muted"}>{kindLabel[item.kind]}</Badge>
                    <span class="selectable mono truncate text-xs text-muted">{item.label}</span>
                    {#each item.found_by as by}<Badge tone="muted" class="h-5">{by}</Badge>{/each}
                    <span class="ml-auto text-xs text-faint tabular-nums">{item.tokens} tokens</span>
                  </div>
                  <p class="selectable mt-1.5 line-clamp-3 text-sm text-fg/90">{item.text}</p>
                </li>
              {/each}
            </ul>
          {:else}
            <p class="text-sm text-muted">Memory added nothing to this run: nothing relevant was found, or it was switched off.</p>
          {/if}
          {#if context.segments?.length}
            <p class="mt-4 mb-1.5 text-xs font-medium text-muted">What the first request held{context.token_factor && context.token_factor !== 1 ? ` (tokens × ${context.token_factor.toFixed(2)}, learned from this model)` : ""}</p>
            <div class="flex flex-wrap gap-1.5">
              {#each context.segments as segment, i (i)}
                <Badge tone={segment.kept ? "muted" : "warn"}>{segment.origin} · {segment.tokens}{segment.kept ? "" : " · left out"}</Badge>
              {/each}
            </div>
          {/if}
        </Card>
      {/if}

      <Card title="What happened" icon="history">
        <Timeline events={store.events} />
      </Card>

      {#if detail.tool_calls.length}
        <Card title="Tools" icon="wrench" padding="none">
          <List class="rounded-none border-0 shadow-none" label="Tool calls">
            {#each detail.tool_calls as call (call.call_id)}
              <ListRow>
                <span class="mono shrink-0 text-sm font-medium">{call.tool}</span>
                <span class="selectable mono min-w-0 flex-1 truncate text-xs text-muted">{JSON.stringify(call.arguments)}</span>
                <Badge tone={callTone(call.status)} dot>{statusLabel(call.status)}</Badge>
              </ListRow>
            {/each}
          </List>
        </Card>
      {/if}

      {#if modelSteps.length}
        <Card title="Models used" icon="cpu" padding="none">
          <List class="rounded-none border-0 shadow-none" label="Models used">
            {#each modelSteps as step, i (i)}
              <ListRow>
                <span class="text-xs text-faint">step {i + 1}</span>
                <span class="mono min-w-0 flex-1 truncate text-sm">{step.provider ? `${step.provider}/${step.name}` : step.name}</span>
                <span class="mono text-xs text-muted tabular-nums">{duration(step.duration_ms)}</span>
              </ListRow>
            {/each}
          </List>
          {#snippet footer()}
            <span class="flex items-center gap-1.5 text-xs {detail.summary.fallbacks > 0 ? 'text-warn' : 'text-muted'}">
              <Icon name={detail.summary.fallbacks > 0 ? "route" : "check"} size={13} />
              {detail.summary.fallbacks > 0
                ? `Switched model ${detail.summary.fallbacks} time(s) because one was unavailable.`
                : "No fallback was needed."}
            </span>
          {/snippet}
        </Card>
      {/if}
      <p class="text-xs text-faint">Run {shortId(run.id)} · asked for <span class="mono">{run.request.model}</span></p>
    </div>
  </div>
{:else}
  <div class="mx-auto max-w-4xl space-y-4 p-6" aria-busy="true" aria-label="Loading">
    <div class="skeleton h-6 w-24"></div>
    <div class="skeleton h-8 w-2/3"></div>
    <div class="grid grid-cols-4 gap-3"><div class="skeleton h-20"></div><div class="skeleton h-20"></div><div class="skeleton h-20"></div><div class="skeleton h-20"></div></div>
  </div>
{/if}
