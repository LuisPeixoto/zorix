<script lang="ts">
  import type { TaskView } from "../../api.ts";
  import { type Scale, buildTimeline } from "../../project.ts";
  import { Chip, Icon, Timeline } from "../../ui/index.ts";
  import WhoAvatar from "./WhoAvatar.svelte";

  /** The tasks across time. The scale is chosen in the page's header; tasks with no dates are listed under the grid. */
  let { views, today, scale, onopen }: { views: TaskView[]; today: string; scale: Scale; onopen: (key: string) => void } = $props();

  const timeline = $derived(buildTimeline(views, scale, today));
  const hasArrows = $derived(views.some((v) => v.task.depends_on.length > 0));
</script>

<Timeline {timeline} {onopen}>
  {#snippet avatar(view)}{#if view.task.assignee}<WhoAvatar who={view.task.assignee} size="xs" />{/if}{/snippet}
</Timeline>

<!-- The arrows are a drawing; this says the same in words for a screen reader. -->
{#if hasArrows}
  <ul class="sr-only" aria-label="What waits for what">
    {#each timeline.bars.filter((b) => b.view.task.depends_on.length > 0) as bar (bar.view.task.key)}
      <li>{bar.view.task.key} waits for {bar.view.task.depends_on.join(" and ")}{timeline.bars.some((o) => bar.view.task.depends_on.includes(o.view.task.key) && o.end >= bar.start) ? ", and starts before one of them is due" : ""}.</li>
    {/each}
  </ul>
{/if}

{#if timeline.unscheduled.length > 0}
  <div class="mt-3 flex flex-wrap items-center gap-2 text-sm text-muted">
    <b class="font-semibold text-fg">Not scheduled · {timeline.unscheduled.length}</b>
    {#each timeline.unscheduled as view (view.task.key)}
      <Chip onclick={() => onopen(view.task.key)}><span class="mono mr-1.5 text-2xs text-faint">{view.task.key}</span>{view.task.title}</Chip>
    {/each}
    <span>Give a task a start and a due date to place it.</span>
  </div>
{/if}
<p class="mt-3 flex items-center gap-1.5 text-sm text-muted">
  <Icon name="alert" size={13} class="shrink-0" />
  <span>Bars fill with progress.{hasArrows ? " An arrow joins a task to the one that waits for it; a red arrow means the later task starts before the first is due." : ""}</span>
</p>
