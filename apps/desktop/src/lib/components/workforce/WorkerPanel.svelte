<script lang="ts">
  import type { ActivityLine, Overview } from "../../api.ts";
  import { store } from "../../stores/runs.svelte.ts";
  import { workforce } from "../../stores/workforce.svelte.ts";
  import { shorten } from "../../format.ts";
  import { Avatar, AutonomyDial, Badge, Button, Icon, IconButton, Sparkbar, Stat } from "../../ui/index.ts";
  import { evalText, levelsUnder, limitsSentence, lineTime, money, onTimeText } from "../../workforce.ts";
  import AssignTaskModal from "./AssignTaskModal.svelte";

  let { overview, id, onclose }: { overview: Overview; id: string; onclose: () => void } = $props();

  const you = $derived(id === "you");
  const view = $derived(overview.workers.find((w) => w.worker.agent === id) ?? null);
  const boss = $derived(view?.worker.reports_to ? (overview.workers.find((w) => w.worker.agent === view.worker.reports_to) ?? null) : null);
  let assigning = $state(false);
  let level = $state<"suggest" | "ask" | "notify" | "budgeted" | "full">("ask");

  // The dial shows what the worker has, and a change is saved at once or put back with the reason in words.
  $effect(() => {
    if (view) level = view.worker.autonomy;
  });
  const top = $derived(levelsUnder(boss?.worker.autonomy ?? null).at(-1) ?? "full");

  async function choose(wanted: typeof level) {
    if (!view || wanted === view.worker.autonomy) return;
    const done = await workforce.edit(view.worker.agent, (f) => ({ ...f, autonomy: wanted }));
    if (!done) level = view.worker.autonomy;
  }

  const mine = $derived(workforce.activity.filter((l) => l.source === id).slice(0, 4));
  const ICON: Record<ActivityLine["kind"], string> = { suggestion: "sparkles", unattended: "zap", waiting: "shield", failed: "alert", plan: "calendar", report: "book", budget: "dollar", org: "users" };
  const share = $derived(view?.worker.weekly_budget_micros ? view.figures.week_micros / view.worker.weekly_budget_micros : null);
</script>

<aside class="flex min-h-0 flex-col overflow-y-auto border-l border-line bg-panel" aria-label={you ? "You" : (view?.name ?? "Worker")}>
  {#if you}
    <header class="flex items-center gap-2.5 border-b border-line-soft px-4 py-3.5">
      <Avatar human size="lg" label="You" />
      <span class="min-w-0 flex-1"><span class="block text-lg font-semibold">You</span><span class="block text-sm text-muted">Owner · approves</span></span>
      <IconButton icon="x" label="Close" onclick={onclose} />
    </header>
    <div class="flex flex-col gap-4 p-4">
      <div class="grid grid-cols-2 gap-2">
        <Stat compact label="To review" value={overview.you.to_review} />
        <Stat compact label="Approvals" value={overview.you.approvals} />
      </div>
      <p class="text-sm text-muted">Every chain ends at you. What waits for you is in the Inbox: approvals to answer and tasks to review.</p>
      <Button icon="bell" class="self-start" onclick={() => store.showInbox()}>Open the Inbox</Button>
    </div>
  {:else if view}
    {@const w = view.worker}
    <header class="flex items-center gap-2.5 border-b border-line-soft px-4 py-3.5">
      <Avatar emoji={view.avatar.emoji} color={view.avatar.color} status={view.status} size="lg" label={view.name} />
      <span class="min-w-0 flex-1">
        <span class="block truncate text-lg font-semibold">{view.name}</span>
        <span class="block text-sm text-muted">{w.role} · reports to {boss ? boss.name : "you"}</span>
      </span>
      {#if view.figures.waiting > 0}<Badge tone="warn" dot>{view.figures.waiting} waiting</Badge>{/if}
      <IconButton icon="x" label="Close" onclick={onclose} />
    </header>
    <div class="flex flex-col gap-5 p-4">
      <div class="grid grid-cols-3 gap-2">
        <Stat compact label="Tasks" value={view.figures.tasks} />
        <Stat compact label={w.manager || view.reports.length > 0 ? "On time" : view.own.eval !== null ? "Eval" : "On time"} value={w.manager || view.reports.length > 0 || view.own.eval === null ? onTimeText(view.figures.on_time_done, view.figures.on_time_all) : evalText(view.own.eval)} />
        <Stat compact label="Week" value={money(view.figures.week_micros)} />
      </div>

      <section class="flex flex-col gap-2" aria-label="Autonomy">
        <h3 class="text-2xs font-semibold tracking-wider text-faint uppercase">Autonomy</h3>
        <AutonomyDial bind:level max={top} label="Autonomy for {view.name}" onchange={(l) => void choose(l)} />
        <p class="min-h-8 text-sm text-muted">{limitsSentence({ ...w, autonomy: level })}</p>
      </section>

      <section class="flex flex-col gap-2" aria-label="This week">
        <h3 class="text-2xs font-semibold tracking-wider text-faint uppercase">This week</h3>
        {#if mine.length === 0}
          <p class="text-sm text-muted">Nothing yet this week.</p>
        {:else}
          <ul class="flex flex-col gap-2">
            {#each mine as line (line.id)}
              <li class="flex items-center gap-2 text-sm">
                <Icon name={ICON[line.kind]} size={14} class="shrink-0 text-muted" />
                <span class="min-w-0 flex-1 truncate" title={line.text}>{shorten(line.text, 60)}</span>
                {#if line.kind === "waiting"}<Badge tone="warn" class="h-4.5">you</Badge>{:else}<span class="tnum shrink-0 text-xs text-faint">{lineTime(line.at, store.now)}</span>{/if}
              </li>
            {/each}
          </ul>
        {/if}
      </section>

      {#if w.weekly_budget_micros !== null && share !== null}
        <section class="flex flex-col gap-1.5" aria-label="Budget">
          <h3 class="text-2xs font-semibold tracking-wider text-faint uppercase">Budget</h3>
          <span class="flex items-center gap-3"><Sparkbar value={share * 100} max={100} label="Budget used" tone={share >= 1 ? "err" : share >= 0.8 ? "warn" : "accent"} class="h-2 flex-1" /><span class="tnum text-sm">{money(view.figures.week_micros)} of {money(w.weekly_budget_micros)}</span></span>
        </section>
      {/if}

      <div class="flex flex-wrap gap-2">
        <Button size="sm" icon="message" onclick={() => store.showAgents(w.agent, "chat")}>Open chat</Button>
        <Button size="sm" icon="plus" onclick={() => (assigning = true)}>Assign a task</Button>
        <Button size="sm" variant="ghost" icon="maximize" onclick={() => store.showWorkforce("chart", w.agent, true)}>Open page</Button>
      </div>
    </div>
    <AssignTaskModal bind:open={assigning} agent={w.agent} name={view.name} />
  {/if}
</aside>
