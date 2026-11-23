<script lang="ts">
  import { t } from "../i18n/index.ts";
  import { ago, shorten } from "../format.ts";
  import { integrations } from "../stores/integrations.svelte.ts";
  import { inbox } from "../stores/inbox.svelte.ts";
  import { store } from "../stores/runs.svelte.ts";
  import { workforce } from "../stores/workforce.svelte.ts";
  import { NOTICE_ICON } from "../workforce.ts";
  import { Badge, Button, EmptyState, Icon, PageHeader } from "../ui/index.ts";

  const items = $derived(inbox.all);

  // What the workers did that is worth a look, kept apart from what waits for a decision.
  $effect(() => {
    void workforce.refreshNotices();
    const timer = setInterval(() => void workforce.refreshNotices(), 6000);
    return () => clearInterval(timer);
  });
  const notices = $derived(workforce.notices);

  function openNotice(n: (typeof notices)[number]) {
    if (n.run) void store.select(n.run);
    else if (n.worker) store.showWorkforce("chart", n.worker, true);
    else store.showWorkforce("activity");
  }

  function review(item: (typeof items)[number]) {
    if (item.kind === "approval") void store.select(item.root);
    else if (item.kind === "workflow") store.showWorkflows("run", null, item.run);
    else if (item.kind === "task") store.showProjects(item.key.slice(0, item.key.lastIndexOf("-")), "board", item.key);
    else {
      store.showIntegrations("connected");
      void integrations.open(item.server);
    }
  }
</script>

<div class="h-full overflow-y-auto">
  <div class="mx-auto max-w-[1100px] p-6">
    <PageHeader title={t("common.inbox")} subtitle={t("inbox.everythingThatIsWaiting")} glow />

    {#if items.length === 0 && notices.length === 0}
      <EmptyState icon="shield-check" title={t("inbox.nothingIsWaitingFor")} text={t("inbox.whenARunWants")} />
    {:else}
      {#if items.length === 0}
        <p class="mb-4 text-sm text-muted">{t("inbox.nothingIsWaitingFor2")}</p>
      {:else}
      <section aria-label={t("common.waitingForYou")} class="overflow-hidden rounded-2xl border border-line bg-panel shadow-card">
        <header class="flex items-center gap-2 border-b border-line-soft bg-panel-2 px-4 py-3">
          <Icon name="shield" size={15} class="text-accent" />
          <h2 class="flex-1 text-base font-semibold">{t("common.waitingForYou")}</h2>
          <Badge tone="warn">{items.length}</Badge>
        </header>
        <ul class="divide-y divide-line-soft">
          {#each items as item (item.id)}
            <li class="flex items-center gap-3 px-4 py-3">
              <span class="grid size-8 shrink-0 place-items-center rounded-lg border border-warn/40 bg-warn/8 text-warn"><Icon name={item.kind === "approval" ? "shield" : item.kind === "workflow" ? "workflow" : item.kind === "task" ? "kanban" : "plug"} size={15} /></span>
              {#if item.kind === "approval"}
                <span class="min-w-0 flex-1">
                  <span class="flex items-baseline gap-2 truncate"><span class="mono text-base font-semibold">{item.tool}</span>{#if item.target}<span class="mono truncate text-sm text-muted">{item.target}</span>{/if}</span>
                  <span class="mt-0.5 block truncate text-xs text-muted">
                    {t("inbox.waiting", { value: item.agent ? `${item.agent} · ` : "", value2: shorten(item.rootPrompt, 60) })} <span class="tnum">{ago(item.at, store.now).replace(" ago", "")}</span>
                  </span>
                </span>
                {#if item.depth > 0}<Badge tone="muted">level {item.depth}</Badge>{/if}
              {:else if item.kind === "workflow"}
                <span class="min-w-0 flex-1">
                  <span class="flex items-baseline gap-2 truncate"><span class="text-base font-semibold">{item.title}</span><span class="truncate text-sm text-muted">{item.what === "question" ? t("inbox.asksAQuestion") : item.what === "action" ? t("inbox.wantsToAct") : t("inbox.needsApproval")}</span></span>
                  <span class="mt-0.5 block truncate text-xs text-muted">
                    {t("inbox.waiting", { value: item.workflow ? `${item.workflow} · ` : "", value2: shorten(item.message, 70) })} <span class="tnum">{ago(item.at, store.now).replace(" ago", "")}</span>
                  </span>
                </span>
              {:else if item.kind === "task"}
                <span class="min-w-0 flex-1">
                  <span class="flex items-baseline gap-2 truncate"><span class="mono text-sm font-semibold text-faint">{item.key}</span><span class="truncate text-base font-semibold">{item.title}</span></span>
                  <span class="mt-0.5 block truncate text-xs text-muted">
                    {item.project} · {item.why === "review" ? t("inbox.readyForYourReview") : item.why === "run_failed" ? t("inbox.itsRunFailed") : t("inbox.itsRunWaitsFor")} · <span class="tnum">{ago(item.at, store.now).replace(" ago", "")}</span> ago
                  </span>
                </span>
              {:else}
                <span class="min-w-0 flex-1">
                  <span class="block truncate text-base font-semibold">{item.server}</span>
                  <span class="mt-0.5 block truncate text-xs text-muted">
                    {item.why === "login" ? t("inbox.needsYouToSign") : t("inbox.toolsToReviewBefore", { count: item.tools })}
                  </span>
                </span>
              {/if}
              <Button size="sm" aria-label={item.kind === "approval" ? t("inbox.review", { tool: item.tool, value: item.target ? ` ${item.target}` : "" }) : item.kind === "workflow" ? t("inbox.review2", { title: item.title }) : item.kind === "task" ? t("inbox.open", { key: item.key }) : t("inbox.review3", { server: item.server })} onclick={() => review(item)}>{t("common.review")}</Button>
            </li>
          {/each}
        </ul>
        <footer class="border-t border-line-soft bg-panel-2 px-4 py-2.5 text-xs text-muted">{t("inbox.oldestAsksFirstAnswers")}</footer>
      </section>
      {/if}

      {#if notices.length > 0}
        <section aria-label={t("inbox.forYourInformation")} class="mt-6 overflow-hidden rounded-2xl border border-line bg-panel shadow-card">
          <header class="flex items-center gap-2 border-b border-line-soft bg-panel-2 px-4 py-3">
            <Icon name="building" size={15} class="text-accent" />
            <h2 class="flex-1 text-base font-semibold">{t("inbox.fromYourWorkforce")}</h2>
            <Badge tone="muted">{notices.length}</Badge>
            <Button size="sm" variant="ghost" icon="check" onclick={() => void workforce.markSeen()}>{t("inbox.markAllRead")}</Button>
          </header>
          <ul class="divide-y divide-line-soft">
            {#each notices as n (n.id)}
              <li class="flex items-center gap-3 px-4 py-3">
                <span class="grid size-8 shrink-0 place-items-center rounded-lg border border-line bg-hover text-muted"><Icon name={NOTICE_ICON[n.kind]} size={15} /></span>
                <span class="min-w-0 flex-1">
                  <span class="block truncate text-base font-semibold">{n.title}</span>
                  <span class="mt-0.5 block truncate text-xs text-muted">{shorten(n.text, 90)} · <span class="tnum">{ago(n.at, store.now)}</span></span>
                </span>
                <Button size="sm" aria-label={t("inbox.open2", { title: n.title })} onclick={() => openNotice(n)}>{t("common.open")}</Button>
              </li>
            {/each}
          </ul>
          <footer class="border-t border-line-soft bg-panel-2 px-4 py-2.5 text-xs text-muted">{t("inbox.nothingHereWaitsFor")}</footer>
        </section>
      {/if}
    {/if}
  </div>
</div>
