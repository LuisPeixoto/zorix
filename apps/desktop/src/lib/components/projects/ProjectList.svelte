<script lang="ts">
  import { t } from "../../i18n/index.ts";
  import type { TaskState, TaskView } from "../../api.ts";
  import { LANE_NAME, LANE_TONE, PRIORITY, type SortBy, cardStatus, dayLabel, dueLabel, sortTasks } from "../../project.ts";
  import { projects } from "../../stores/projects.svelte.ts";
  import { Badge, Button, EmptyState, Icon, Sparkbar } from "../../ui/index.ts";
  import WhoAvatar from "./WhoAvatar.svelte";

  /** The same tasks as the board, as a table that is read and sorted by column. */
  let {
    views,
    manager,
    today,
    onopen,
    onadd,
  }: { views: TaskView[]; manager: string | null; today: string; onopen: (key: string) => void; onadd: (lane: TaskState) => void } = $props();

  let by = $state<SortBy>("key");
  let ascending = $state(true);

  const rows = $derived(sortTasks(views, by, ascending));

  function sort(column: SortBy) {
    if (by === column) ascending = !ascending;
    else {
      by = column;
      ascending = true;
    }
  }

  const COLUMNS: { id: SortBy | null; label: string }[] = [
    { id: "key", label: t("projects.projectList.key") },
    { id: null, label: t("common.task") },
    { id: "priority", label: t("common.priority") },
    { id: null, label: t("projects.projectList.assignee") },
    { id: "state", label: t("projects.projectList.state") },
    { id: "progress", label: t("common.progress") },
    { id: "due", label: t("common.due") },
  ];
</script>

{#if views.length === 0}
  <EmptyState icon="kanban" title={t("projects.projectList.noTasksHere")} text={t("projects.projectList.addATaskOr")}>
    {#snippet actions()}<Button variant="primary" icon="plus" onclick={() => onadd("backlog")}>{t("common.addTask")}</Button>{/snippet}
  </EmptyState>
{:else}
  <div class="overflow-hidden rounded-2xl border border-line bg-panel shadow-card">
    <div class="overflow-x-auto">
      <table class="w-full min-w-[820px] text-left text-base">
        <thead>
          <tr class="border-b border-line-soft bg-panel-2 text-2xs font-semibold tracking-wider text-faint uppercase">
            {#each COLUMNS as column (column.label)}
              <th scope="col" class="px-3.5 py-2.5 font-semibold whitespace-nowrap" aria-sort={column.id && by === column.id ? (ascending ? "ascending" : "descending") : undefined}>
                {#if column.id}
                  {@const id = column.id}
                  <button type="button" class="inline-flex items-center gap-1 rounded uppercase outline-none hover:text-fg focus-visible:text-fg" onclick={() => sort(id)}>
                    {column.label}
                    {#if by === column.id}<Icon name={ascending ? "chevron-up" : "chevron-down"} size={12} />{/if}
                  </button>
                {:else}{column.label}{/if}
              </th>
            {/each}
          </tr>
        </thead>
        <tbody>
          {#each rows as view (view.task.key)}
            {@const task = view.task}
            {@const due = dueLabel(task.due, today)}
            {@const status = cardStatus(view, manager, projects.agentName)}
            <tr class="cursor-pointer border-b border-line-soft last:border-b-0 hover:bg-hover" onclick={() => onopen(task.key)}>
              <td class="mono px-3.5 py-2.5 text-xs whitespace-nowrap text-faint">
                <button type="button" class="rounded outline-none focus-visible:ring-2 focus-visible:ring-accent/60" onclick={(e) => { e.stopPropagation(); onopen(task.key); }} aria-label={t("projects.projectList.open", { key: task.key })}>{task.key}</button>
              </td>
              <td class="px-3.5 py-2.5 font-medium">
                {task.title}
                {#if status}<Badge tone={status.tone} live={status.live} class="ml-1.5 h-4.5 align-middle">{status.text}</Badge>{/if}
              </td>
              <td class="px-3.5 py-2.5">
                <span class="inline-flex items-center gap-1 text-xs font-semibold {PRIORITY[task.priority].text}"><Icon name="flag" size={11} />{PRIORITY[task.priority].label}</span>
              </td>
              <td class="px-3.5 py-2.5">
                {#if task.assignee}
                  <span class="flex items-center gap-2"><WhoAvatar who={task.assignee} /><span class="whitespace-nowrap">{projects.face(task.assignee).label}</span></span>
                {:else}<span class="text-faint">—</span>{/if}
              </td>
              <td class="px-3.5 py-2.5"><Badge tone={LANE_TONE[task.state]}>{LANE_NAME[task.state]}</Badge></td>
              <td class="w-40 px-3.5 py-2.5">
                <span class="flex items-center gap-2"><Sparkbar value={task.progress} max={100} label={t("common.progress")} class="flex-1" /><span class="tnum w-9 text-right text-xs text-muted">{task.progress}%</span></span>
              </td>
              <td class="px-3.5 py-2.5 whitespace-nowrap">
                {#if due && task.state !== "done"}
                  <span class="inline-flex items-center gap-1 text-sm {due.late ? 'font-medium text-err' : 'text-muted'}"><Icon name={due.late ? "clock" : "calendar"} size={12} />{due.text}</span>
                {:else if task.due}<span class="text-sm text-muted">{dayLabel(task.due)}</span>{:else}<span class="text-faint">—</span>{/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  </div>
  <div class="mt-3 flex items-center justify-between">
    <Button variant="ghost" size="sm" icon="plus" onclick={() => onadd("backlog")}>{t("common.addTask")}</Button>
    <span class="text-xs text-muted">{views.length} {views.length === 1 ? "task" : "tasks"}</span>
  </div>
{/if}
