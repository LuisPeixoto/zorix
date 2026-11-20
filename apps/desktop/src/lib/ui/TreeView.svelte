<script lang="ts">
  import { tick } from "svelte";

  import type { ExecutionState } from "../api.ts";
  import { stateLabel, stateTone } from "../format.ts";
  import { type TreeNode, costShares, elapsed, flatten } from "../tree.ts";
  import Avatar from "./Avatar.svelte";
  import Badge from "./Badge.svelte";
  import Icon from "./Icon.svelte";
  import Sparkbar from "./Sparkbar.svelte";
  import { cn } from "./cn.ts";

  /** Every execution of one piece of work, indented by who asked whom: face, agent, state, steps, time and cost. Arrow keys move,
   *  Right and Left open and close a branch, Enter picks one. */
  let {
    root,
    selected = null,
    collapsed = $bindable(new Set<string>()),
    now = Date.now(),
    look = () => ({ emoji: "🤖", color: "muted", name: "Run" }),
    onselect,
    class: klass = "",
  }: {
    root: TreeNode;
    selected?: string | null;
    collapsed?: Set<string>;
    now?: number;
    /** The face and name to show for a run. */
    look?: (run: TreeNode["run"]) => { emoji: string; color: string; name: string };
    onselect?: (id: string) => void;
    class?: string;
  } = $props();

  const rows = $derived(flatten(root, collapsed));
  const shares = $derived(costShares(root));
  let active = $state<string | null>(null);
  let list: HTMLUListElement | undefined = $state();

  const focused = $derived(active && rows.some((r) => r.node.run.id === active) ? active : (selected && rows.some((r) => r.node.run.id === selected) ? selected : root.run.id));

  function toggle(id: string, open: boolean) {
    const next = new Set(collapsed);
    if (open) next.delete(id);
    else next.add(id);
    collapsed = next;
  }

  async function focus(id: string) {
    active = id;
    await tick();
    list?.querySelector<HTMLElement>(`[data-id="${CSS.escape(id)}"]`)?.focus();
  }

  function onkey(event: KeyboardEvent, index: number) {
    const row = rows[index];
    const parentIndex = () => {
      for (let i = index - 1; i >= 0; i--) if (rows[i].depth < row.depth) return i;
      return -1;
    };
    switch (event.key) {
      case "ArrowDown":
        if (index + 1 < rows.length) void focus(rows[index + 1].node.run.id);
        break;
      case "ArrowUp":
        if (index > 0) void focus(rows[index - 1].node.run.id);
        break;
      case "Home":
        void focus(rows[0].node.run.id);
        break;
      case "End":
        void focus(rows[rows.length - 1].node.run.id);
        break;
      case "ArrowRight":
        if (row.hasChildren && !row.expanded) toggle(row.node.run.id, true);
        else if (row.expanded) void focus(rows[index + 1].node.run.id);
        break;
      case "ArrowLeft":
        if (row.expanded) toggle(row.node.run.id, false);
        else if (parentIndex() >= 0) void focus(rows[parentIndex()].node.run.id);
        break;
      case "Enter":
      case " ":
        onselect?.(row.node.run.id);
        break;
      default:
        return;
    }
    event.preventDefault();
  }

  const clock = (ms: number | null) => (ms === null ? "–" : ms < 1000 ? `${Math.round(ms)} ms` : ms < 60_000 ? `${(ms / 1000).toFixed(ms < 10_000 ? 1 : 0)} s` : `${Math.floor(ms / 60_000)} min ${Math.round((ms % 60_000) / 1000)} s`);
  const money = (micros: number) => (micros === 0 ? "free" : `$${(micros / 1_000_000).toFixed(4)}`);
</script>

<ul bind:this={list} role="tree" aria-label="Runs in this work" class={cn("overflow-hidden rounded-2xl border border-line bg-panel", klass)}>
  {#each rows as row, index (row.node.run.id)}
    {@const run = row.node.run}
    {@const face = look(run)}
    {@const isSelected = selected === run.id}
    <li role="none" class="border-t border-line-soft first:border-t-0">
      <div
        role="treeitem"
        tabindex={focused === run.id ? 0 : -1}
        aria-level={row.depth + 1}
        aria-selected={isSelected}
        aria-expanded={row.hasChildren ? row.expanded : undefined}
        data-id={run.id}
        onclick={() => { active = run.id; onselect?.(run.id); }}
        onkeydown={(e) => onkey(e, index)}
        class={cn(
          "group flex cursor-pointer items-center gap-2.5 py-2 pr-3 transition duration-150 outline-none hover:bg-hover focus-visible:bg-hover focus-visible:ring-2 focus-visible:ring-accent focus-visible:ring-inset",
          isSelected && "bg-accent-soft shadow-[inset_3px_0_0_var(--accent)]",
        )}
        style:padding-left="{12 + row.depth * 22}px"
      >
        {#if row.hasChildren}
          <button type="button" tabindex="-1" class="grid size-5 shrink-0 place-items-center rounded-md text-faint transition hover:bg-line hover:text-fg" aria-label={row.expanded ? `Hide what ${face.name} started` : `Show what ${face.name} started`} onclick={(e) => { e.stopPropagation(); toggle(run.id, !row.expanded); }}>
            <Icon name={row.expanded ? "chevron-down" : "chevron-right"} size={14} />
          </button>
        {:else}
          <span class="size-5 shrink-0" aria-hidden="true"></span>
        {/if}
        <Avatar emoji={face.emoji} color={face.color} size="sm" />
        <span class="min-w-0 flex-1">
          <span class="flex items-baseline gap-2">
            <span class="truncate text-md font-medium">{face.name}</span>
            {#if run.agent}<span class="mono shrink-0 text-xs text-faint">v{run.agent.version}</span>{/if}
          </span>
          <span class="block truncate text-sm text-muted">{run.request.prompt}</span>
        </span>
        <span class="hidden w-20 shrink-0 text-right text-xs text-muted tabular-nums sm:block">{run.usage.model_calls} step{run.usage.model_calls === 1 ? "" : "s"}</span>
        <span class="hidden w-16 shrink-0 text-right text-xs text-muted tabular-nums md:block">{clock(elapsed(run, now))}</span>
        <span class="hidden w-24 shrink-0 items-center justify-end gap-2 text-xs text-muted tabular-nums md:flex">
          <Sparkbar value={shares.get(run.id) ?? 0} label="Share of the cost of {face.name}" />
          {money(run.usage.cost_micros)}
        </span>
        <Badge tone={stateTone(run.state as ExecutionState)} live={["running", "waiting_for_model", "waiting_for_tool", "waiting_for_children", "queued"].includes(run.state)} dot>{stateLabel(run.state as ExecutionState)}</Badge>
      </div>
    </li>
  {/each}
</ul>
