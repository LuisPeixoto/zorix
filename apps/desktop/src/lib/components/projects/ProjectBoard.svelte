<script lang="ts">
  import type { TaskState, TaskView } from "../../api.ts";
  import { LANES, PRIORITY, byLane, cardStatus, dueLabel, linkLabel } from "../../project.ts";
  import { projects } from "../../stores/projects.svelte.ts";
  import { AvatarStack, TaskBoard, TaskCard } from "../../ui/index.ts";
  import WhoAvatar from "./WhoAvatar.svelte";

  /** The board: a card per task in four lanes. A drop says which lane and place; the rules (a run starting, a run stopping) are Rust's. */
  let {
    views,
    manager,
    today,
    onopen,
    onadd,
  }: { views: TaskView[]; manager: string | null; today: string; onopen: (key: string) => void; onadd: (lane: TaskState) => void } = $props();

  const items = $derived(
    LANES.flatMap((lane) => byLane(views)[lane.id].map((view) => ({ id: view.task.key, lane: lane.id as string, label: view.task.key, view }))),
  );
</script>

<TaskBoard
  lanes={LANES}
  {items}
  onopen={(item) => onopen(item.id)}
  onadd={(lane) => onadd(lane as TaskState)}
  onmove={(item, lane, index) => void projects.move(item.id, lane as TaskState, index)}
>
  {#snippet card(item, dragging, open)}
    {@const task = item.view.task}
    {@const pr = task.links.map((l) => linkLabel(l.url, l.title)).find((l) => l.kind === "pr")}
    <TaskCard
      taskKey={task.key}
      title={task.title}
      priority={PRIORITY[task.priority]}
      due={task.state === "done" ? null : dueLabel(task.due, today)}
      progress={task.state === "in_progress" ? task.progress : null}
      deps={task.depends_on.length}
      deliverables={task.state === "done" ? task.deliverables.length : 0}
      pull={pr?.text ?? null}
      status={cardStatus(item.view, manager, projects.agentName)}
      {dragging}
      selected={projects.taskKey === task.key}
      onopen={open}
    >
      {#snippet assignees()}
        {#if task.assignee}<AvatarStack><WhoAvatar who={task.assignee} /></AvatarStack>{/if}
      {/snippet}
    </TaskCard>
  {/snippet}
</TaskBoard>
