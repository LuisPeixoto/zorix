<script lang="ts">
  import { t } from "../../i18n/index.ts";
  import { ago } from "../../format.ts";
  import { teamRemove, errorText } from "../../api.ts";
  import { agents } from "../../stores/agents.svelte.ts";
  import { store } from "../../stores/runs.svelte.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { Avatar, Badge, Button, Callout, Card, EmptyState, Icon, IconButton, Modal } from "../../ui/index.ts";
  import StatusBadge from "../StatusBadge.svelte";
  import TeamEditor from "./TeamEditor.svelte";

  let { slug }: { slug: string } = $props();
  const view = $derived(agents.team(slug));
  const team = $derived(view?.team);
  let editorOpen = $state(false);
  let confirmOpen = $state(false);
  let removing = $state(false);

  const runs = $derived(store.runs.filter((r) => r.agent?.team === slug).slice(0, 8));

  async function remove() {
    removing = true;
    try {
      await teamRemove(slug);
      toast.ok(t("agents.teamPage.theTeamWasDeleted", { slug }));
      confirmOpen = false;
      await agents.openTeams(null);
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      removing = false;
    }
  }
</script>

<div class="h-full overflow-y-auto">
  <div class="mx-auto max-w-4xl p-8">
    <button type="button" class="mb-4 inline-flex items-center gap-1 text-sm text-muted transition hover:text-fg" onclick={() => agents.openTeams(null)}>
      <Icon name="chevron-right" size={14} class="rotate-180" /> {t("agents.teamPage.allTeams")}
    </button>
    {#if team && view}
      <header class="flex items-start gap-4">
        <Avatar emoji={team.avatar.emoji} color={team.avatar.color} size="xl" label={team.name} />
        <div class="min-w-0 flex-1">
          <h1 class="text-3xl font-semibold tracking-tight">{team.name}</h1>
          <p class="mono mt-0.5 text-xs text-faint">{team.slug}</p>
          <p class="mt-1.5 max-w-2xl text-md text-muted">{team.description || t("common.noDescriptionYet")}</p>
        </div>
        <div class="flex shrink-0 items-center gap-1.5">
          <Button variant="primary" icon="message" disabled={!view.lead || view.missing.length > 0} onclick={() => store.showAgents(team.lead, "chat", null, team.slug)}>{t("agents.teamPage.chatWithTheTeam")}</Button>
          <IconButton icon="edit" label={t("agents.teamPage.editTheTeam")} onclick={() => (editorOpen = true)} />
          <IconButton icon="trash" label={t("agents.teamPage.deleteTheTeam")} onclick={() => (confirmOpen = true)} />
        </div>
      </header>

      {#if view.missing.length}
        <Callout tone="warn" title={t("agents.teamPage.someAgentsAreGone")} class="mt-5">{t("agents.teamPage.areNotAnAgent", { value: view.missing.join(", ") })}</Callout>
      {/if}

      <section class="mt-8" aria-labelledby="lead-title">
        <h2 id="lead-title" class="mb-3 text-lg font-semibold">{t("agents.teamPage.howItWorks")}</h2>
        <div class="grid items-stretch gap-4 md:grid-cols-[minmax(0,1fr)_auto_minmax(0,2fr)]">
          <Card title={t("agents.teamPage.lead")} icon="sparkles">
            {#if view.lead}
              <button type="button" class="flex w-full items-center gap-3 text-left" onclick={() => agents.open(view.lead!.slug)}>
                <Avatar emoji={view.lead.emoji} color={view.lead.color} size="lg" />
                <span class="min-w-0"><span class="block truncate text-md font-semibold">{view.lead.name}</span><span class="block text-xs text-muted">{t("agents.teamPage.receivesTheWork")}</span></span>
              </button>
            {:else}<p class="text-sm text-muted">{t("agents.teamPage.isNotAnAgent", { lead: team.lead })}</p>{/if}
          </Card>
          <div class="hidden items-center text-faint md:flex" aria-hidden="true"><Icon name="branch" size={22} /></div>
          <Card title={t("agents.teamPage.asksAtTheSame")} icon="users">
            <ul class="grid gap-2 sm:grid-cols-2">
              {#each view.members as m (m.slug)}
                <li>
                  <button type="button" class="flex w-full items-center gap-2.5 rounded-xl border border-line bg-panel-2 p-2.5 text-left transition hover:border-line-strong" onclick={() => agents.open(m.slug)}>
                    <Avatar emoji={m.emoji} color={m.color} size="sm" />
                    <span class="min-w-0"><span class="block truncate text-md font-medium">{m.name}</span><span class="block truncate text-xs text-muted">{m.description}</span></span>
                  </button>
                </li>
              {:else}
                <li class="text-sm text-muted">{t("agents.teamPage.noMembersYet")}</li>
              {/each}
            </ul>
          </Card>
        </div>
        <p class="mt-3 text-xs text-faint">{t("agents.teamPage.eachMemberGetsOnly")}</p>
      </section>

      <section class="mt-8" aria-labelledby="runs-title">
        <h2 id="runs-title" class="mb-3 text-lg font-semibold">{t("agents.teamPage.recentWork")}</h2>
        {#if runs.length === 0}
          <EmptyState icon="history" title={t("agents.teamPage.nothingYet")} text={t("agents.teamPage.startAChatWith")} />
        {:else}
          <ul class="space-y-2">
            {#each runs as r (r.id)}
              <li>
                <button type="button" class="raised flex w-full items-center gap-3 rounded-xl border border-line bg-panel px-3.5 py-2.5 text-left transition hover:border-line-strong" onclick={() => { store.showRuns(); void store.select(r.id); }}>
                  <span class="min-w-0 flex-1 truncate text-md">{r.request.prompt}</span>
                  <span class="tnum text-xs text-muted">{ago(r.created_at, store.now)}</span>
                  <StatusBadge state={r.state} />
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    {:else}
      <Callout tone="err" title={t("agents.teamPage.thisTeamCouldNot")}>{t("agents.teamPage.itMayHaveBeen")} <button type="button" class="underline" onclick={() => agents.openTeams(null)}>{t("agents.teamPage.backToTheTeams")}</button></Callout>
    {/if}
  </div>
</div>

<TeamEditor bind:open={editorOpen} editing={slug} />

<Modal bind:open={confirmOpen} title={t("agents.teamPage.deleteThisTeam")} icon="trash" size="sm">
  <p class="text-base text-muted"><strong class="text-fg">{team?.name}</strong> {t("agents.teamPage.isDeletedTheAgents")}</p>
  {#snippet footer()}
    <Button onclick={() => (confirmOpen = false)}>{t("common.cancel")}</Button>
    <Button variant="danger" icon="trash" loading={removing} onclick={remove}>{t("common.delete")}</Button>
  {/snippet}
</Modal>
