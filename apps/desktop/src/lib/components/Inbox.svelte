<script lang="ts">
  import { ago, shorten } from "../format.ts";
  import { integrations } from "../stores/integrations.svelte.ts";
  import { inbox } from "../stores/inbox.svelte.ts";
  import { store } from "../stores/runs.svelte.ts";
  import { Badge, Button, EmptyState, Icon, PageHeader } from "../ui/index.ts";

  const items = $derived(inbox.all);

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
    <PageHeader title="Inbox" subtitle="Everything that is waiting for a decision from you. Nothing that changes something happens until you answer." glow />

    {#if items.length === 0}
      <EmptyState icon="shield-check" title="Nothing is waiting for you" text="When a run wants to change something, a workflow stops for you, a task is ready for review, or a connected server needs a review or a sign-in, it appears here, oldest first." />
    {:else}
      <section aria-label="Waiting for you" class="overflow-hidden rounded-2xl border border-line bg-panel shadow-card">
        <header class="flex items-center gap-2 border-b border-line-soft bg-panel-2 px-4 py-3">
          <Icon name="shield" size={15} class="text-accent" />
          <h2 class="flex-1 text-base font-semibold">Waiting for you</h2>
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
                    {item.agent ? `${item.agent} · ` : ""}{shorten(item.rootPrompt, 60)} · waiting <span class="tnum">{ago(item.at, store.now).replace(" ago", "")}</span>
                  </span>
                </span>
                {#if item.depth > 0}<Badge tone="muted">level {item.depth}</Badge>{/if}
              {:else if item.kind === "workflow"}
                <span class="min-w-0 flex-1">
                  <span class="flex items-baseline gap-2 truncate"><span class="text-base font-semibold">{item.title}</span><span class="truncate text-sm text-muted">{item.what === "question" ? "asks a question" : item.what === "action" ? "wants to act" : "needs approval"}</span></span>
                  <span class="mt-0.5 block truncate text-xs text-muted">
                    {item.workflow ? `${item.workflow} · ` : ""}{shorten(item.message, 70)} · waiting <span class="tnum">{ago(item.at, store.now).replace(" ago", "")}</span>
                  </span>
                </span>
              {:else if item.kind === "task"}
                <span class="min-w-0 flex-1">
                  <span class="flex items-baseline gap-2 truncate"><span class="mono text-sm font-semibold text-faint">{item.key}</span><span class="truncate text-base font-semibold">{item.title}</span></span>
                  <span class="mt-0.5 block truncate text-xs text-muted">
                    {item.project} · {item.why === "review" ? "ready for your review" : item.why === "run_failed" ? "its run failed" : "its run waits for an approval"} · <span class="tnum">{ago(item.at, store.now).replace(" ago", "")}</span> ago
                  </span>
                </span>
              {:else}
                <span class="min-w-0 flex-1">
                  <span class="block truncate text-base font-semibold">{item.server}</span>
                  <span class="mt-0.5 block truncate text-xs text-muted">
                    {item.why === "login" ? "Needs you to sign in" : `${item.tools} tool${item.tools === 1 ? "" : "s"} to review before an agent can use ${item.tools === 1 ? "it" : "them"}`}
                  </span>
                </span>
              {/if}
              <Button size="sm" aria-label={item.kind === "approval" ? `Review ${item.tool}${item.target ? ` ${item.target}` : ""}` : item.kind === "workflow" ? `Review ${item.title}` : item.kind === "task" ? `Open ${item.key}` : `Review ${item.server}`} onclick={() => review(item)}>Review</Button>
            </li>
          {/each}
        </ul>
        <footer class="border-t border-line-soft bg-panel-2 px-4 py-2.5 text-xs text-muted">Oldest asks first. Answers are bound to the exact call.</footer>
      </section>
    {/if}
  </div>
</div>
