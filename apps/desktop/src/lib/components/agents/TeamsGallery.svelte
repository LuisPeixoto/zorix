<script lang="ts">
  import { t as tr } from "../../i18n/index.ts";
  import { agents } from "../../stores/agents.svelte.ts";
  import { Avatar, Badge, Button, Callout, Card, EmptyState, Icon, Skeleton } from "../../ui/index.ts";
  import TeamEditor from "./TeamEditor.svelte";

  let adding = $state<string | null>(null);
  let editorOpen = $state(false);

  async function add(id: string) {
    adding = id;
    const slug = await agents.addTeamTemplate(id);
    adding = null;
    if (slug) void agents.openTeams(slug);
  }

  const free = $derived(agents.teamTemplates.filter((t) => !t.installed));
  const look = (slug: string) => agents.card(slug);
</script>

<div class="mb-4 flex items-center justify-between gap-3">
  <p class="max-w-2xl text-base text-muted">{tr("agents.teamsGallery.aTeamIsA")}</p>
  <Button icon="plus" onclick={() => (editorOpen = true)} disabled={agents.cards.length < 2}>{tr("agents.teamsGallery.newTeam")}</Button>
</div>

{#if !agents.loaded}
  <div class="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">{#each [0, 1] as i (i)}<Skeleton class="h-40 rounded-2xl" />{/each}</div>
{:else}
  {#if agents.teams.length === 0}
    <EmptyState icon="users" title={tr("agents.teamsGallery.noTeamsYet")} text={tr("agents.teamsGallery.addAReadyMade")} />
  {:else}
    <ul class="grid gap-4 sm:grid-cols-2 lg:grid-cols-3" aria-label={tr("agents.teamsGallery.yourTeams")}>
      {#each agents.teams as t (t.team.slug)}
        <li>
          <button type="button" onclick={() => agents.openTeams(t.team.slug)} class="group shadow-card flex h-full w-full flex-col rounded-2xl border border-line bg-panel p-4 text-left transition duration-150 ease-out hover:-translate-y-px hover:border-line-strong hover:shadow-3">
            <span class="flex items-start gap-3">
              <Avatar emoji={t.team.avatar.emoji} color={t.team.avatar.color} size="lg" />
              <span class="min-w-0 flex-1">
                <span class="block truncate text-lg font-semibold">{t.team.name}</span>
                <span class="mono block truncate text-xs text-faint">{t.team.slug}</span>
              </span>
              <Icon name="chevron-right" size={16} class="mt-1 text-faint transition group-hover:translate-x-0.5 group-hover:text-accent" />
            </span>
            <span class="mt-3 line-clamp-2 min-h-[2.6em] text-base text-muted">{t.team.description || tr("common.noDescriptionYet")}</span>
            <span class="mt-4 flex items-center gap-2">
              <span class="flex -space-x-2" aria-hidden="true">
                {#if t.lead}<Avatar emoji={t.lead.emoji} color={t.lead.color} size="sm" class="ring-2 ring-panel" />{/if}
                {#each t.members.slice(0, 4) as m (m.slug)}<Avatar emoji={m.emoji} color={m.color} size="sm" class="ring-2 ring-panel" />{/each}
              </span>
              <span class="text-xs text-muted">{tr("agents.teamsGallery.ledByAsks", { lead: t.lead?.name ?? t.team.lead, length: t.team.members.length })}</span>
              {#if t.missing.length}<Badge tone="warn">{tr("agents.teamsGallery.needsAttention")}</Badge>{/if}
            </span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}

  {#if free.length}
    <section class="mt-10" aria-labelledby="team-templates-title">
      <h2 id="team-templates-title" class="mb-3 text-xl font-semibold">{tr("agents.teamsGallery.addAReadyMade2")}</h2>
      <div class="grid gap-4 sm:grid-cols-2">
        {#each free as t (t.id)}
          <Card interactive>
            <div class="flex items-start gap-3">
              <Avatar emoji={t.emoji} color={t.color} size="lg" />
              <div class="min-w-0 flex-1">
                <h3 class="text-md font-semibold">{t.name}</h3>
                <p class="mt-0.5 text-sm text-muted">{t.description}</p>
                <p class="mt-2 flex flex-wrap items-center gap-1.5 text-xs text-faint"><Icon name="users" size={12} /> {t.lead} asks {t.members.join(", ")}</p>
              </div>
              <Button size="sm" variant="subtle" icon="plus" loading={adding === t.id} onclick={() => add(t.id)} aria-label={tr("common.add", { name: t.name })}>{tr("common.add")}</Button>
            </div>
          </Card>
        {/each}
      </div>
      <p class="mt-3 text-xs text-faint">{tr("agents.teamsGallery.theAgentsOfThe")}</p>
    </section>
  {/if}
  {#if agents.cards.length < 2 && free.length === 0}<Callout tone="muted" title={tr("agents.teamsGallery.youNeedTwoAgents")}>{tr("agents.teamsGallery.aTeamNeedsA")}</Callout>{/if}
{/if}

<TeamEditor bind:open={editorOpen} editing={null} />
