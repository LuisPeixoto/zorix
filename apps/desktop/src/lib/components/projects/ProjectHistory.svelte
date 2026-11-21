<script lang="ts">
  import { type ProjectEvent, projectHistory } from "../../api.ts";
  import { type EventGroup, clock, eventGroup, eventSentence, groupByDay, todayOf } from "../../project.ts";
  import { projects } from "../../stores/projects.svelte.ts";
  import { store } from "../../stores/runs.svelte.ts";
  import { Badge, Chip, EmptyState, Icon } from "../../ui/index.ts";
  import WhoAvatar from "./WhoAvatar.svelte";

  /** Everything that happened to the project's tasks, newest first, in sentences. */
  let { project, onopen }: { project: string; onopen: (key: string) => void } = $props();

  let events = $state<ProjectEvent[]>([]);
  let loaded = $state(false);
  let filter = $state<EventGroup | "all">("all");

  $effect(() => {
    // The board refreshes every few seconds while a project is open; history is read when it does.
    void projects.board?.tasks.length;
    void projectHistory(project, 200)
      .then((list) => {
        events = list;
        loaded = true;
      })
      .catch(() => (loaded = true));
  });

  const FILTERS: { id: EventGroup | "all"; label: string }[] = [
    { id: "all", label: "All" },
    { id: "moves", label: "Moves" },
    { id: "runs", label: "Runs" },
    { id: "approvals", label: "Approvals" },
  ];

  const shown = $derived(events.filter((e) => filter === "all" || eventGroup(e) === filter));
  const groups = $derived(groupByDay(shown, todayOf(store.now)));

  function badge(event: ProjectEvent): { text: string; tone: "ok" } | null {
    const k = event.kind;
    if (k.type === "moved" && k.approved_by) return { text: `Approved by ${k.approved_by}`, tone: "ok" };
    return null;
  }
</script>

<div class="mb-4 flex justify-end">
  <div class="flex items-center gap-2" role="group" aria-label="Kinds of events">
    {#each FILTERS as f (f.id)}<Chip selected={filter === f.id} onclick={() => (filter = f.id)}>{f.label}</Chip>{/each}
  </div>
</div>

{#if loaded && shown.length === 0}
  <EmptyState icon="history" title="Nothing here yet" text={filter === "all" ? "Moves, runs, comments and approvals appear here as they happen." : "No event of this kind yet."} />
{:else}
  <div class="flex flex-col gap-4">
    {#each groups as group (group.label)}
      <section aria-label={group.label}>
        <h3 class="mb-1.5 px-0.5 text-2xs font-semibold tracking-wider text-faint uppercase">{group.label}</h3>
        <ul class="divide-y divide-line-soft overflow-hidden rounded-2xl border border-line bg-panel shadow-card" role="list">
          {#each group.events as event (event.seq)}
            {@const mark = badge(event)}
            <li class="flex items-center gap-3 px-3.5 py-2.5">
              {#if event.by}
                <WhoAvatar who={event.by} size="md" />
              {:else}
                <span class="grid size-9 shrink-0 place-items-center rounded-xl border border-line bg-panel-2 text-muted" title="ZORIX"><Icon name="activity" size={16} /></span>
              {/if}
              <div class="min-w-0 flex-1">
                <p class="text-base">
                  {#if event.task}
                    {@const sentence = eventSentence(event, projects.agentName)}
                    {@const at = sentence.indexOf(event.task)}
                    {#if at >= 0}{sentence.slice(0, at)}<button type="button" class="mono rounded text-sm font-semibold outline-none hover:text-accent focus-visible:underline" onclick={() => onopen(event.task ?? "")}>{event.task}</button>{sentence.slice(at + event.task.length)}{:else}{sentence}{/if}
                  {:else}{eventSentence(event, projects.agentName)}{/if}
                </p>
                <p class="text-xs text-muted">{clock(event.at)}</p>
              </div>
              {#if mark}<Badge tone={mark.tone}>{mark.text}</Badge>{/if}
            </li>
          {/each}
        </ul>
      </section>
    {/each}
  </div>
{/if}
