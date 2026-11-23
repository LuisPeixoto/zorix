<script lang="ts">
  import { t } from "../i18n/index.ts";
  import { onMount } from "svelte";

  import { type OrgNode, fitScale, layoutChart } from "../workforce.ts";
  import Avatar from "./Avatar.svelte";
  import WorkerCard from "./WorkerCard.svelte";
  import { cn } from "./cn.ts";

  /**
   * The workforce as an organisation: you at the top, then who answers to you and who answers to them, as cards joined by elbow
   * connectors, and everyone below as small chips. A person's card is dashed with a round avatar. The chart shrinks to fit the room it
   * has (design system: OrgChart).
   */
  let { nodes, selected = null, onselect, class: klass = "" }: { nodes: OrgNode[]; selected?: string | null; onselect?: (id: string) => void; class?: string } = $props();

  const chart = $derived(layoutChart(nodes.map((n) => ({ id: n.id, parent: n.parent }))));
  const byId = $derived(new Map(nodes.map((n) => [n.id, n])));
  let room = $state(0);
  let box: HTMLElement | undefined = $state();
  const scale = $derived(fitScale(chart.width, room));

  onMount(() => {
    if (!box) return;
    const observer = new ResizeObserver(([entry]) => (room = entry.contentRect.width - 8));
    observer.observe(box);
    return () => observer.disconnect();
  });
</script>

<div bind:this={box} class={cn("overflow-hidden", klass)} role="group" aria-label={t("ui.orgChart.organisationChart")}>
  <div class="relative mx-auto" style:width="{chart.width * scale}px" style:height="{chart.height * scale}px">
    <div class="absolute top-0 left-0 origin-top-left" style:width="{chart.width}px" style:height="{chart.height}px" style:transform="scale({scale})">
      <svg class="absolute inset-0" width={chart.width} height={chart.height} fill="none" stroke="var(--line-strong)" stroke-width="1.5" aria-hidden="true">
        {#each chart.paths as d (d)}<path {d} />{/each}
      </svg>
      {#each chart.nodes as placed (placed.id)}
        {@const n = byId.get(placed.id)}
        {#if n}
          <div class="absolute" style:left="{placed.x}px" style:top="{placed.y}px" style:width="{placed.w}px" style:height="{placed.h}px">
            {#if placed.chip}
              <button
                type="button"
                aria-pressed={selected === n.id}
                aria-label={n.label}
                onclick={() => onselect?.(n.id)}
                class={cn(
                  "flex size-full items-center gap-1.5 rounded-full border border-line bg-panel px-2 text-left text-xs font-medium shadow-1 outline-none hover:border-line-strong focus-visible:ring-2 focus-visible:ring-accent/60",
                  selected === n.id && "border-accent ring-2 ring-accent/30",
                )}
              >
                <Avatar emoji={n.emoji} color={n.color} status={n.status} size="xs" class="size-4.5 rounded-md" />
                <span class="truncate">{n.name}</span>
              </button>
            {:else}
              <WorkerCard
                name={n.name}
                role={n.role}
                emoji={n.emoji}
                color={n.color}
                status={n.status}
                human={n.human}
                figures={n.figures}
                selected={selected === n.id}
                label={n.label}
                onclick={() => onselect?.(n.id)}
              />
            {/if}
          </div>
        {/if}
      {/each}
    </div>
  </div>
</div>
