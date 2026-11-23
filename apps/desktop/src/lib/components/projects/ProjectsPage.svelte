<script lang="ts">
  import { t } from "../../i18n/index.ts";
  import { dueLabel, progressOf, todayOf } from "../../project.ts";
  import { projects } from "../../stores/projects.svelte.ts";
  import { store } from "../../stores/runs.svelte.ts";
  import { AvatarStack, Badge, Button, Callout, Chip, EmptyState, Icon, PageHeader, Sparkbar } from "../../ui/index.ts";
  import NewProjectModal from "./NewProjectModal.svelte";
  import WhoAvatar from "./WhoAvatar.svelte";

  let creating = $state(false);
  const today = $derived(todayOf(store.now));
  const cards = $derived(projects.cards);
  const needing = $derived(cards.reduce((sum, c) => sum + c.needs_you, 0));

  // The list is alive: runs end and tasks move while it is open.
  $effect(() => {
    void projects.loadPeople();
    void projects.refreshList();
    const timer = setInterval(() => void projects.refreshList(), 3000);
    return () => clearInterval(timer);
  });

  function show(archived: boolean) {
    projects.showArchived = archived;
    void projects.refreshList();
  }
</script>

<div class="h-full overflow-y-auto">
  <div class="mx-auto max-w-[1100px] p-6">
    <PageHeader title={t("common.projects")} subtitle={t("projects.projectsPage.tasksForYouAnd")} glow>
      {#snippet actions()}
        <Button variant="primary" icon="plus" onclick={() => (creating = true)}>{t("projects.projectsPage.newProject")}</Button>
      {/snippet}
    </PageHeader>

    {#if projects.error && cards.length === 0}
      <Callout tone="err" title={t("projects.projectsPage.projectsCouldNotBe")}>{projects.error}</Callout>
    {:else}
      <div class="mb-4 flex flex-wrap items-center justify-between gap-3">
        <div class="flex items-center gap-2" role="group" aria-label={t("projects.projectsPage.whichProjects")}>
          <Chip selected={!projects.showArchived} onclick={() => show(false)}>{t("projects.projectsPage.active")}</Chip>
          <Chip selected={projects.showArchived} onclick={() => show(true)}>{t("projects.projectsPage.archived")}</Chip>
        </div>
        <span class="text-sm text-muted">{cards.length} {cards.length === 1 ? "project" : "projects"}{needing > 0 ? ` · ${needing} ${needing === 1 ? t("projects.projectsPage.taskNeeds") : t("projects.projectsPage.tasksNeed")} you` : ""}</span>
      </div>

      {#if cards.length === 0}
        <EmptyState
          icon="kanban"
          title={projects.showArchived ? t("projects.projectsPage.noArchivedProjects") : t("projects.projectsPage.noProjectsYet")}
          text={projects.showArchived ? t("projects.projectsPage.aProjectYouArchive") : t("projects.projectsPage.aProjectHoldsTasks")}
        >
          {#snippet actions()}
            {#if !projects.showArchived}<Button variant="primary" icon="plus" onclick={() => (creating = true)}>{t("projects.projectsPage.newProject")}</Button>{/if}
          {/snippet}
        </EmptyState>
      {:else}
        <ul class="grid items-start gap-4 sm:grid-cols-2 xl:grid-cols-3" role="list">
          {#each cards as card (card.project.id)}
            {@const p = card.project}
            {@const progress = progressOf(card.counts)}
            {@const due = dueLabel(p.due, today)}
            {@const managerFace = p.manager ? projects.face({ type: "agent", slug: p.manager }) : null}
            <li>
              <button
                type="button"
                onclick={() => store.showProjects(p.key)}
                class="flex w-full flex-col gap-3 rounded-2xl border border-line bg-panel p-4 text-left shadow-card transition duration-150 ease-out outline-none hover:-translate-y-px hover:border-line-strong hover:shadow-2 focus-visible:ring-2 focus-visible:ring-accent/60"
              >
                <span class="flex min-h-5.5 items-center justify-between">
                  <span class="mono text-xs font-medium text-faint">{p.key}</span>
                  {#if card.needs_you > 0}<Badge tone="warn">{t("projects.projectsPage.needYou", { needs_you: card.needs_you })}</Badge>{/if}
                </span>
                <span>
                  <span class="block text-lg font-semibold">{p.name}</span>
                  {#if p.description}<span class="mt-0.5 line-clamp-2 block text-sm leading-snug text-muted">{p.description}</span>{/if}
                </span>
                <span class="flex flex-col gap-1.5">
                  <span class="flex items-center justify-between text-xs"><span class="text-muted">{t("common.progress")}</span><b class="tnum font-semibold">{progress.text}</b></span>
                  <Sparkbar value={progress.percent} max={100} label={t("common.progress")} class="w-full" />
                </span>
                <span class="flex items-center justify-between gap-2 border-t border-line-soft pt-3 text-sm">
                  <span class="flex min-w-0 items-center gap-1.5 text-muted">
                    {#if managerFace}
                      <WhoAvatar who={{ type: "agent", slug: p.manager ?? "" }} />
                      <span class="truncate">{managerFace.label}</span>
                    {:else}
                      {t("projects.projectsPage.noManager")}
                    {/if}
                  </span>
                  <span class="flex items-center gap-2.5">
                    {#if due}
                      <span class="inline-flex items-center gap-1 text-xs {due.late ? 'font-medium text-err' : 'text-muted'}"><Icon name={due.late ? "clock" : "calendar"} size={12} />{due.text}</span>
                    {/if}
                    <AvatarStack extra={Math.max(0, card.assignees.length - 4)}>
                      {#each card.assignees.slice(0, 4) as who (JSON.stringify(who))}<WhoAvatar {who} />{/each}
                    </AvatarStack>
                  </span>
                </span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    {/if}
  </div>
</div>

<NewProjectModal bind:open={creating} />
