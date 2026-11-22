<script lang="ts">
  import { type OrgReport, errorText, workerFire, workerPlan, workerReport } from "../../api.ts";
  import { dayLabel } from "../../project.ts";
  import { store } from "../../stores/runs.svelte.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { workforce } from "../../stores/workforce.svelte.ts";
  import { usdToMicros } from "../../project.ts";
  import { usd } from "../../shell.ts";
  import { Avatar, AutonomyDial, Badge, Button, Callout, Chip, EmptyState, Icon, Input, LogStream, Markdown, MetricTile, ToggleRow } from "../../ui/index.ts";
  import { STATUS_TEXT, evalText, levelsUnder, limitsSentence, money, onTimeText, spendDelta } from "../../workforce.ts";
  import AssignTaskModal from "./AssignTaskModal.svelte";
  import HireWorkerModal from "./HireWorkerModal.svelte";

  const detail = $derived(workforce.detail);
  const view = $derived(detail?.view ?? null);
  const overview = $derived(workforce.overview);
  const boss = $derived(view?.worker.reports_to ? (overview?.workers.find((w) => w.worker.agent === view.worker.reports_to) ?? null) : null);
  const top = $derived(levelsUnder(boss?.worker.autonomy ?? null).at(-1) ?? "full");

  let editing = $state(false);
  let assigning = $state(false);
  let confirmFire = $state(false);
  let level = $state<"suggest" | "ask" | "notify" | "budgeted" | "full">("ask");
  let budget = $state("");
  let minutes = $state("120");
  let patterns = $state<string[]>([]);
  let pattern = $state("");
  let busy = $state(false);
  let seen = $state("");

  // The form starts from the worker, and starts again only when somebody else changed it.
  $effect(() => {
    if (!view) return;
    const w = view.worker;
    const key = `${w.agent}:${w.revision}`;
    if (key === seen) return;
    seen = key;
    level = w.autonomy;
    budget = w.weekly_budget_micros ? String(w.weekly_budget_micros / 1_000_000) : "";
    minutes = String(Math.round(w.task_time_ms / 60_000));
    patterns = [...w.still_ask];
  });

  $effect(() => {
    void workforce.loadAgents();
    void workforce.refresh();
    void workforce.refreshDetail();
    const timer = setInterval(() => {
      void workforce.refreshDetail();
      void workforce.refresh();
    }, 4000);
    return () => clearInterval(timer);
  });

  const micros = $derived(usdToMicros(budget));
  const time = $derived(Number(minutes));
  const capError = $derived.by(() => {
    const cap = boss?.worker.weekly_budget_micros ?? null;
    if (micros === undefined) return "Write an amount in dollars, such as 25.";
    if (cap === null) return null;
    if (micros === null) return `${boss?.name} has a weekly budget of ${money(cap)}, so this one needs one too.`;
    return micros > cap ? `Up to ${money(cap)}, the budget of ${boss?.name}.` : null;
  });
  const timeError = $derived(!Number.isFinite(time) || time < 1 || time > 1440 ? "Between 1 and 1440 minutes." : null);
  const dirty = $derived(
    view !== null &&
      (level !== view.worker.autonomy || (micros ?? null) !== view.worker.weekly_budget_micros || Math.round(time) * 60_000 !== view.worker.task_time_ms || patterns.join("\n") !== view.worker.still_ask.join("\n")),
  );

  async function save() {
    if (!view || !dirty || capError || timeError || busy) return;
    busy = true;
    const ok = await workforce.edit(view.worker.agent, (f) => ({ ...f, autonomy: level, weekly_budget_micros: micros ?? null, task_time_ms: Math.round(time) * 60_000, still_ask: patterns }));
    busy = false;
    if (ok) toast.ok("Saved.");
  }

  function addPattern() {
    const p = pattern.trim();
    if (p && !patterns.includes(p)) patterns = [...patterns, p];
    pattern = "";
  }

  async function toggleSchedule(on: boolean) {
    if (!view) return;
    await workforce.edit(view.worker.agent, (f) => ({ ...f, schedule_enabled: on }));
  }

  async function ask(kind: "plan" | "report") {
    if (!view || busy) return;
    busy = true;
    try {
      const started = kind === "plan" ? await workerPlan(view.worker.agent) : await workerReport(view.worker.agent);
      toast.ok(kind === "plan" ? `${view.name} is planning the week.` : `${view.name} is writing the report.`);
      await workforce.refreshDetail();
      void store.select(started.run);
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      busy = false;
    }
  }

  async function fire() {
    if (!view) return;
    busy = true;
    try {
      await workerFire(view.worker.agent);
      toast.ok(`${view.name} was let go.`);
      store.showWorkforce("chart");
    } catch (e) {
      toast.err(errorText(e));
      confirmFire = false;
    } finally {
      busy = false;
    }
  }

  const delta = $derived(detail ? spendDelta(view?.own.week_micros ?? 0, detail.spend_last_week_micros) : null);
  const lines = $derived((detail?.activity ?? []).map((l) => ({ id: l.id, at: l.at, level: l.level, source: l.source, text: l.text, run: l.run })));
  const dept = $derived(overview?.settings.departments.find((d) => d.slug === view?.worker.department)?.name ?? view?.worker.department ?? "");
  const latest = $derived<OrgReport | null>(detail?.reports.find((r) => r.status === "done") ?? null);
  const dayName = (week: string) => dayLabel(week);

  function openRun(line: { run?: string | null }) {
    if (line.run) void store.select(line.run);
  }
</script>

<div class="h-full overflow-y-auto">
  <div class="mx-auto max-w-[1100px] p-6">
    {#if !view}
      {#if workforce.error}
        <Callout tone="err" title="The worker could not be read">{workforce.error}</Callout>
        <Button class="mt-3" icon="chevron-left" onclick={() => store.showWorkforce("chart")}>Back to the workforce</Button>
      {:else}
        <p class="text-muted" role="status"><Icon name="loader" size={14} class="mr-1.5 inline animate-spin" />Reading the worker…</p>
      {/if}
    {:else}
      {@const w = view.worker}
      <nav aria-label="Where you are" class="mb-1 flex items-center gap-1.5 text-sm text-muted">
        <button type="button" class="inline-flex items-center gap-1.5 rounded outline-none hover:text-fg focus-visible:ring-2 focus-visible:ring-accent/60" onclick={() => store.showWorkforce("chart")}><Icon name="building" size={13} />Workforce</button>
        <Icon name="chevron-right" size={12} /><span aria-current="page" class="text-fg">{view.name}</span>
      </nav>
      <header class="mb-5 flex flex-wrap items-start gap-4">
        <Avatar emoji={view.avatar.emoji} color={view.avatar.color} status={view.status} size="lg" label={view.name} />
        <div class="min-w-0 flex-1">
          <h1 class="flex flex-wrap items-center gap-2.5 text-3xl font-semibold tracking-tight">
            {view.name}
            {#if view.status}<Badge tone={view.status === "ok" ? "ok" : view.status === "warn" ? "warn" : "err"} dot live={view.status === "ok"}>{STATUS_TEXT[view.status]}</Badge>{/if}
          </h1>
          <p class="mt-1 text-base text-muted">{w.manager ? "Virtual manager" : w.role} · {dept} · reports to {boss ? boss.name : "you"}{view.description ? `. ${view.description}` : ""}</p>
        </div>
        <div class="flex flex-wrap items-center gap-2">
          <Button icon="edit" onclick={() => (editing = true)}>Edit</Button>
          <Button icon="message" onclick={() => store.showAgents(w.agent, "chat")}>Open chat</Button>
          <Button variant="primary" icon="plus" onclick={() => (assigning = true)}>Assign a task</Button>
        </div>
      </header>

      <div class="mb-4 grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
        <MetricTile icon="kanban" label="Tasks" value={String(view.figures.tasks)} detail="{view.own.tasks} {view.own.tasks === 1 ? 'is' : 'are'} its own" color={1} />
        <MetricTile icon="clock" label="On time" value={onTimeText(view.figures.on_time_done, view.figures.on_time_all).replace("%", "")} unit={view.figures.on_time_all > 0 ? "%" : undefined} detail={view.figures.on_time_all > 0 ? `last 30 days · ${view.figures.on_time_done} of ${view.figures.on_time_all}` : "no task with a due date finished in 30 days"} color={2} />
        <MetricTile icon="flask" label="Eval score" value={evalText(view.own.eval)} detail={view.own.eval === null ? `no evaluation has run for ${view.name}` : "latest finished evaluation"} color={3} />
        <MetricTile icon="dollar" label="Spend this week" value={usd(view.own.week_micros).replace(/\.00$/, "")} {delta} detail={w.weekly_budget_micros !== null ? `of ${money(w.weekly_budget_micros)} budget · resets Monday` : "no weekly budget · resets Monday"} series={detail?.spend_days} seriesLabel="Spend each of the last 7 days, in micro-dollars" color={4} />
      </div>

      <div class="grid items-start gap-4 lg:grid-cols-[minmax(0,1.15fr)_minmax(0,1fr)]">
        <div class="flex min-w-0 flex-col gap-4">
          <section class="flex flex-col gap-3 rounded-2xl border border-line bg-panel p-4 shadow-card" aria-label="Autonomy">
            <div class="flex items-baseline justify-between gap-3">
              <h2 class="text-2xs font-semibold tracking-wider text-faint uppercase">Autonomy</h2>
              <span class="text-xs text-muted">{boss ? `It can only be as free as ${boss.name}` : "It answers to you, so any level"}</span>
            </div>
            <AutonomyDial bind:level max={top} label="Autonomy for {view.name}" />
            <p class="text-sm text-muted">{limitsSentence({ autonomy: level, weekly_budget_micros: micros ?? null, task_time_ms: Math.round(time) * 60_000 || w.task_time_ms, still_ask: patterns })} Calls that ask only by default run without asking at Notify and above, and are listed in Activity. Tools that always ask still ask.</p>
            <div class="grid grid-cols-2 gap-3">
              <label class="flex flex-col gap-1 text-sm font-medium">Budget per week
                <Input bind:value={budget} icon="dollar" inputmode="decimal" placeholder="No limit" invalid={capError !== null} />
                {#if capError}<span class="text-xs font-normal text-err" role="alert">{capError}</span>{/if}
              </label>
              <label class="flex flex-col gap-1 text-sm font-medium">Time per task
                <Input bind:value={minutes} inputmode="numeric" invalid={timeError !== null} />
                <span class="text-xs font-normal {timeError ? 'text-err' : 'text-faint'}">{timeError ?? "minutes of work"}</span>
              </label>
            </div>
            <div class="flex flex-col gap-1.5">
              <span class="text-sm font-medium">Still asks, whatever the level</span>
              <div class="flex flex-wrap items-center gap-2">
                {#each patterns as p (p)}<Chip selected onremove={() => (patterns = patterns.filter((x) => x !== p))}><span class="mono text-xs">{p}</span></Chip>{/each}
                <span class="flex items-center gap-1.5">
                  <Input size="sm" class="w-44" bind:value={pattern} placeholder="repo:acme/web:write" aria-label="An access that always asks" onkeydown={(e) => e.key === "Enter" && addPattern()} />
                  <Button size="sm" variant="ghost" icon="plus" disabled={pattern.trim() === ""} onclick={addPattern}>Add</Button>
                </span>
              </div>
              <span class="text-xs text-faint">Access patterns, like the ones an agent is granted.</span>
            </div>
            <div class="flex items-center justify-between gap-2 border-t border-line-soft pt-3">
              {#if confirmFire}
                <span class="text-sm text-err">Let {view.name} go?</span>
                <span class="flex gap-2"><Button disabled={busy} onclick={() => (confirmFire = false)}>Keep</Button><Button variant="danger" icon="trash" disabled={busy} onclick={() => void fire()}>Let go</Button></span>
              {:else}
                <Button variant="ghost" icon="trash" onclick={() => (confirmFire = true)}>Let go</Button>
                <Button variant="primary" icon="check" disabled={!dirty || capError !== null || timeError !== null || busy} onclick={() => void save()}>Save</Button>
              {/if}
            </div>
          </section>

          <section aria-label="This week" class="flex flex-col gap-2">
            <h2 class="text-2xs font-semibold tracking-wider text-faint uppercase">This week</h2>
            <LogStream {lines} now={store.now} scope={view.name} empty="Nothing yet this week." onopen={openRun} />
          </section>
        </div>

        <div class="flex min-w-0 flex-col gap-4">
          {#if w.manager}
            <section class="flex flex-col gap-2 rounded-2xl border border-line bg-panel p-4 shadow-card" aria-label="Schedule">
              <h2 class="text-2xs font-semibold tracking-wider text-faint uppercase">Schedule</h2>
              <ToggleRow class="-mx-4 border-b border-line-soft" checked={w.schedule.enabled} label="Plan on Mondays at 09:00 and report on Fridays at 16:00" description="Assigns tasks through the project tools. Runs only while the app or `zorix serve` is open." onchange={(on) => void toggleSchedule(on)} />
              <div class="flex flex-wrap gap-2 pt-1">
                <Button size="sm" icon="calendar" disabled={busy || detail?.busy_with === "plan"} onclick={() => void ask("plan")}>{detail?.busy_with === "plan" ? "Planning…" : "Plan the week"}</Button>
                <Button size="sm" icon="book" disabled={busy || detail?.busy_with === "report"} onclick={() => void ask("report")}>{detail?.busy_with === "report" ? "Writing…" : "Write the report"}</Button>
              </div>
            </section>
          {/if}

          <section class="flex flex-col rounded-2xl border border-line bg-panel p-4 shadow-card" aria-label="Open tasks">
            <div class="mb-1.5 flex items-baseline justify-between"><h2 class="text-2xs font-semibold tracking-wider text-faint uppercase">Open tasks</h2><span class="text-xs text-muted tnum">{detail?.tasks.length ?? 0}</span></div>
            {#if (detail?.tasks.length ?? 0) === 0}
              <p class="py-2 text-sm text-muted">No open tasks.</p>
            {:else}
              <ul>
                {#each detail?.tasks ?? [] as t (t.key)}
                  <li class="border-t border-line-soft first:border-t-0">
                    <button type="button" class="flex w-full items-center gap-2 py-2 text-left text-sm outline-none hover:text-accent focus-visible:ring-2 focus-visible:ring-accent/60" onclick={() => store.showProjects(t.key.slice(0, t.key.lastIndexOf("-")), "board", t.key)}>
                      <span class="mono text-xs text-faint">{t.key}</span>
                      <span class="min-w-0 flex-1 truncate">{t.title}</span>
                      {#if t.due}<span class="inline-flex items-center gap-1 text-xs {t.late ? 'font-medium text-err' : 'text-muted'}"><Icon name={t.late ? "clock" : "calendar"} size={12} />{dayName(t.due)}</span>{/if}
                    </button>
                  </li>
                {/each}
              </ul>
            {/if}
          </section>

          {#if view.reports.length > 0}
            <section class="flex flex-col gap-2 rounded-2xl border border-line bg-panel p-4 shadow-card" aria-label="Reports to {view.name}">
              <div class="flex items-baseline justify-between"><h2 class="text-2xs font-semibold tracking-wider text-faint uppercase">Reports to {view.name}</h2><span class="text-xs text-muted">{view.reports.length} {view.reports.length === 1 ? "worker" : "workers"}</span></div>
              {#each view.reports as slug (slug)}
                {@const r = overview?.workers.find((o) => o.worker.agent === slug)}
                {#if r}
                  <button type="button" class="flex items-center gap-2.5 rounded-lg text-left outline-none hover:bg-hover focus-visible:ring-2 focus-visible:ring-accent/60" onclick={() => workforce.openWorker(slug)}>
                    <Avatar emoji={r.avatar.emoji} color={r.avatar.color} status={r.status} size="sm" />
                    <span class="min-w-0 flex-1 text-sm"><b class="font-medium">{r.name}</b> <span class="text-muted">{r.worker.role}</span></span>
                    <Badge tone={r.worker.autonomy === "suggest" ? "muted" : r.worker.autonomy === "ask" ? "info" : "accent"}>{r.worker.autonomy[0].toUpperCase() + r.worker.autonomy.slice(1)}</Badge>
                  </button>
                {/if}
              {/each}
            </section>
          {/if}

          {#if w.manager}
            <section class="flex flex-col gap-1.5 rounded-2xl border border-line bg-panel p-4 shadow-card" aria-label="Latest report">
              <div class="flex items-baseline justify-between"><h2 class="text-2xs font-semibold tracking-wider text-faint uppercase">Latest {latest?.kind === "plan" ? "plan" : "report"}</h2>{#if latest}<span class="text-xs text-muted">Week of {dayName(latest.week)}</span>{/if}</div>
              {#if latest}
                <div class="selectable max-h-72 overflow-y-auto text-sm"><Markdown text={latest.body.trim()} /></div>
              {:else}
                <EmptyState icon="book" title="No plan or report yet" text="Ask {view.name} to plan the week or write the report, or turn its schedule on." />
              {/if}
            </section>
          {/if}
        </div>
      </div>
    {/if}
  </div>
</div>

{#if view}
  <HireWorkerModal bind:open={editing} worker={view} />
  <AssignTaskModal bind:open={assigning} agent={view.worker.agent} name={view.name} />
{/if}
