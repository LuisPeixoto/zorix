<script lang="ts">
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
  <p class="max-w-2xl text-base text-muted">A team is a lead and the agents it may ask. The lead splits the work, asks several at the same time, and puts the answers together.</p>
  <Button icon="plus" onclick={() => (editorOpen = true)} disabled={agents.cards.length < 2}>New team</Button>
</div>

{#if !agents.loaded}
  <div class="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">{#each [0, 1] as i (i)}<Skeleton class="h-40 rounded-2xl" />{/each}</div>
{:else}
  {#if agents.teams.length === 0}
    <Card padding="none"><EmptyState icon="users" title="No teams yet" text="Add a ready-made team below, or make one from agents you already have." /></Card>
  {:else}
    <ul class="grid gap-4 sm:grid-cols-2 lg:grid-cols-3" aria-label="Your teams">
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
            <span class="mt-3 line-clamp-2 min-h-[2.6em] text-base text-muted">{t.team.description || "No description yet."}</span>
            <span class="mt-4 flex items-center gap-2">
              <span class="flex -space-x-2" aria-hidden="true">
                {#if t.lead}<Avatar emoji={t.lead.emoji} color={t.lead.color} size="sm" class="ring-2 ring-panel" />{/if}
                {#each t.members.slice(0, 4) as m (m.slug)}<Avatar emoji={m.emoji} color={m.color} size="sm" class="ring-2 ring-panel" />{/each}
              </span>
              <span class="text-xs text-muted">Led by {t.lead?.name ?? t.team.lead} · asks {t.team.members.length}</span>
              {#if t.missing.length}<Badge tone="warn">needs attention</Badge>{/if}
            </span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}

  {#if free.length}
    <section class="mt-10" aria-labelledby="team-templates-title">
      <h2 id="team-templates-title" class="mb-3 text-xl font-semibold">Add a ready-made team</h2>
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
              <Button size="sm" variant="subtle" icon="plus" loading={adding === t.id} onclick={() => add(t.id)} aria-label={`Add ${t.name}`}>Add</Button>
            </div>
          </Card>
        {/each}
      </div>
      <p class="mt-3 text-xs text-faint">The agents of the team are added too, unless you already have them. An agent you changed is never overwritten.</p>
    </section>
  {/if}
  {#if agents.cards.length < 2 && free.length === 0}<Callout tone="muted" title="You need two agents">A team needs a lead and at least one agent to ask.</Callout>{/if}
{/if}

<TeamEditor bind:open={editorOpen} editing={null} />
