<script lang="ts">
  import type { Deliverable, TaskView } from "../../api.ts";
  import { LANE_NAME, LANE_TONE, clock, dayLabel, todayOf } from "../../project.ts";
  import { projects } from "../../stores/projects.svelte.ts";
  import { store } from "../../stores/runs.svelte.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { Badge, Button, EmptyState, Icon, Markdown } from "../../ui/index.ts";
  import WhoAvatar from "./WhoAvatar.svelte";

  /** What the project produced, by task: an agent's answer, a link, a file. An answer opens in place. */
  let { views, onopen }: { views: TaskView[]; onopen: (key: string) => void } = $props();

  let openId = $state<string | null>(null);
  const groups = $derived(
    views
      .filter((v) => v.task.deliverables.length > 0)
      .map((v) => ({ view: v, items: [...v.task.deliverables].sort((a, b) => b.at - a.at) }))
      .sort((a, b) => (b.items[0]?.at ?? 0) - (a.items[0]?.at ?? 0)),
  );

  const icon = (d: Deliverable) => (d.kind === "link" ? (d.body.includes("/pull/") || d.body.includes("merge_requests") ? "git-branch" : "link") : "file-text");
  const kindWord = (d: Deliverable) => (d.kind === "answer" ? "Answer" : d.kind === "link" ? "Link" : "File");

  function when(at: number): string {
    const day = todayOf(at);
    return day === todayOf(store.now) ? `Today ${clock(at)}` : dayLabel(day);
  }

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      toast.ok("Copied.");
    } catch {
      toast.err("Could not copy.");
    }
  }
</script>

{#if groups.length === 0}
  <EmptyState icon="file-text" title="Nothing produced yet" text="What a run answers, and what you or an agent attach to a task, is listed here. Move an agent's task to In progress to start one." />
{:else}
  <div class="flex flex-col gap-5">
    {#each groups as group (group.view.task.key)}
      {@const task = group.view.task}
      <section aria-label="{task.key}, {task.title}">
        <div class="mb-2 flex items-center gap-2.5">
          <button type="button" class="mono text-xs text-faint outline-none hover:text-fg focus-visible:underline" onclick={() => onopen(task.key)}>{task.key}</button>
          <span class="font-semibold">{task.title}</span>
          <Badge tone={LANE_TONE[task.state]}>{LANE_NAME[task.state]}</Badge>
          {#if task.assignee}<span class="ml-auto"><WhoAvatar who={task.assignee} /></span>{/if}
        </div>
        <ul class="divide-y divide-line-soft overflow-hidden rounded-2xl border border-line bg-panel shadow-card" role="list">
          {#each group.items as d (d.id)}
            <li>
              <div class="flex items-center gap-3 px-3.5 py-2.5">
                <span class="grid size-7.5 shrink-0 place-items-center rounded-lg border border-line bg-panel-2 text-muted"><Icon name={icon(d)} size={15} /></span>
                <div class="min-w-0 flex-1">
                  <p class="truncate text-base font-medium">{d.title}</p>
                  <p class="text-xs text-muted">{kindWord(d)} · {d.by ? projects.face(d.by).label : "ZORIX"} · {when(d.at)}</p>
                </div>
                {#if d.kind === "answer"}
                  <Button variant="ghost" size="sm" iconRight={openId === d.id ? "chevron-up" : "chevron-down"} onclick={() => (openId = openId === d.id ? null : d.id)}>{openId === d.id ? "Hide" : "Open"}</Button>
                {:else}
                  <Button variant="ghost" size="sm" iconRight="copy" onclick={() => void copy(d.body)}>Copy link</Button>
                {/if}
              </div>
              {#if openId === d.id && d.kind === "answer"}
                <div class="border-t border-line-soft bg-panel-2 px-4 py-3"><Markdown text={d.body} /></div>
              {/if}
            </li>
          {/each}
        </ul>
      </section>
    {/each}
  </div>
{/if}
