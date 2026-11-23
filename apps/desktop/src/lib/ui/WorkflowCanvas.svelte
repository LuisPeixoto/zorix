<script lang="ts">
  import { t } from "../i18n/index.ts";
  import type { UiTone } from "./tone.ts";
  import type { WfRun, WorkflowSpec } from "../api.ts";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import WorkflowNode from "./WorkflowNode.svelte";
  import { cn } from "./cn.ts";
  import {
    GRID,
    KINDS,
    type Known,
    NODE_H,
    NODE_W,
    PALETTE_GROUPS,
    addStep,
    connect,
    detailLine,
    disconnect,
    edgeMid,
    edgePath,
    edgeState,
    fit,
    kindInfo,
    kindLine,
    miniMap,
    moveStep,
    nodeState,
    removeStep,
    snap,
    zoomAt,
    type NodeState,
    type View,
  } from "../workflow.ts";

  /**
   * The visual editor of a workflow and, with `readonly` and a `run`, the live view of one (design system: WorkflowCanvas).
   * Steps snap to a 16px grid. Arrows run from the bottom of one step to the top of the next; a step that is going draws its
   * arrow as a moving dash. Drag a step to move it, drag from its bottom dot to another step to join them, drag the background
   * to pan, hold Ctrl or ⌘ and scroll to zoom. Everything has a keyboard equivalent, and the steps are also listed as text.
   */
  let {
    spec,
    selected = $bindable(null),
    selectedEdge = $bindable(null),
    readonly = false,
    run = null,
    known = { agents: [], teams: [], workflows: [] },
    canUndo = false,
    onchange,
    onundo,
    onerror,
    class: klass = "",
  }: {
    spec: WorkflowSpec;
    selected?: string | null;
    selectedEdge?: { from: string; to: string } | null;
    readonly?: boolean;
    run?: WfRun | null;
    known?: Known;
    canUndo?: boolean;
    onchange?: (spec: WorkflowSpec) => void;
    onundo?: () => void;
    onerror?: (message: string) => void;
    class?: string;
  } = $props();

  let host = $state<HTMLDivElement>();
  let width = $state(800);
  let height = $state(600);
  let view = $state<View>({ zoom: 1, panX: 0, panY: 0 });
  let fitted = $state(false);
  let drag = $state<{ id: string; x: number; y: number; moved: boolean } | null>(null);
  let pan = $state<{ startX: number; startY: number; panX: number; panY: number } | null>(null);
  let link = $state<{ from: string; x: number; y: number } | null>(null);
  let menu = $state(false);

  const place = (id: string) => {
    if (drag && drag.id === id) return { x: drag.x, y: drag.y };
    const step = spec.steps.find((s) => s.id === id);
    return step ? { x: step.x, y: step.y } : { x: 0, y: 0 };
  };

  // The first time there is something to see, show all of it.
  $effect(() => {
    if (!fitted && width > 0 && spec.steps.length > 0) {
      view = fit(spec.steps, { w: width, h: height });
      fitted = true;
    }
  });

  const BADGE_TONE: Record<NodeState, UiTone> = { idle: "muted", running: "info", waiting: "warn", done: "ok", failed: "err", skipped: "muted" };
  function runBadge(id: string): { text: string; tone: UiTone; live?: boolean } | undefined {
    const record = run?.steps[id];
    if (!record || record.state === "pending") return undefined;
    const text = record.note || record.error || record.state;
    const state = nodeState(record.state);
    return { text, tone: BADGE_TONE[state], live: state === "waiting" || state === "running" };
  }

  const world = (event: PointerEvent | WheelEvent) => {
    const box = host?.getBoundingClientRect();
    const px = event.clientX - (box?.left ?? 0);
    const py = event.clientY - (box?.top ?? 0);
    return { px, py, x: (px - view.panX) / view.zoom, y: (py - view.panY) / view.zoom };
  };

  function onpointerdown(event: PointerEvent) {
    if (event.button !== 0) return;
    const target = event.target as HTMLElement;
    const port = target.closest<HTMLElement>("[data-port]");
    const node = target.closest<HTMLElement>("[data-node]");
    if (target.closest("[data-chrome]")) return;
    host?.focus({ preventScroll: true });
    menu = false;
    if (node) {
      const id = node.dataset.node as string;
      selected = id;
      selectedEdge = null;
      if (readonly) return;
      host?.setPointerCapture(event.pointerId);
      const at = world(event);
      if (port?.dataset.port === "out") {
        link = { from: id, x: at.x, y: at.y };
      } else {
        const here = place(id);
        drag = { id, x: here.x, y: here.y, moved: false };
        pan = { startX: at.x - here.x, startY: at.y - here.y, panX: 0, panY: 0 };
      }
      return;
    }
    const edge = target.closest<SVGElement>("[data-edge]");
    if (edge) {
      const [from, to] = (edge.dataset.edge as string).split(">");
      selectedEdge = { from, to };
      selected = null;
      return;
    }
    selected = null;
    selectedEdge = null;
    host?.setPointerCapture(event.pointerId);
    pan = { startX: event.clientX, startY: event.clientY, panX: view.panX, panY: view.panY };
  }

  function onpointermove(event: PointerEvent) {
    if (link) {
      const at = world(event);
      link = { ...link, x: at.x, y: at.y };
    } else if (drag && pan) {
      const at = world(event);
      const x = snap(at.x - pan.startX);
      const y = snap(at.y - pan.startY);
      if (x !== drag.x || y !== drag.y) drag = { ...drag, x, y, moved: true };
    } else if (pan) {
      view = { ...view, panX: pan.panX + (event.clientX - pan.startX), panY: pan.panY + (event.clientY - pan.startY) };
    }
  }

  function onpointerup(event: PointerEvent) {
    if (host?.hasPointerCapture(event.pointerId)) host.releasePointerCapture(event.pointerId);
    if (link) {
      const under = document.elementFromPoint(event.clientX, event.clientY)?.closest<HTMLElement>("[data-node]");
      const from = link.from;
      link = null;
      if (under && under.dataset.node !== from) {
        const joined = connect(spec, from, under.dataset.node as string);
        if ("error" in joined) onerror?.(joined.error);
        else onchange?.(joined.spec);
      }
    } else if (drag) {
      const moved = drag;
      drag = null;
      pan = null;
      if (moved.moved) onchange?.(moveStep(spec, moved.id, moved));
    }
    pan = null;
  }

  function onwheel(event: WheelEvent) {
    event.preventDefault();
    if (event.ctrlKey || event.metaKey) {
      const at = world(event);
      view = zoomAt(view, view.zoom * (event.deltaY < 0 ? 1.1 : 1 / 1.1), { x: at.px, y: at.py });
    } else {
      view = { ...view, panX: view.panX - event.deltaX, panY: view.panY - event.deltaY };
    }
  }

  function remove() {
    if (readonly) return;
    if (selectedEdge) {
      onchange?.(disconnect(spec, selectedEdge.from, selectedEdge.to));
      selectedEdge = null;
    } else if (selected) {
      const removed = removeStep(spec, selected);
      if ("error" in removed) onerror?.(removed.error);
      else {
        onchange?.(removed.spec);
        selected = null;
      }
    }
  }

  function onkeydown(event: KeyboardEvent) {
    const target = event.target as HTMLElement;
    if (target.closest("[data-chrome]") && target.tagName !== "DIV") return;
    const key = event.key;
    const focused = target.closest<HTMLElement>("[data-node]");
    if ((key === "Enter" || key === " ") && focused) {
      event.preventDefault();
      selected = focused.dataset.node ?? null;
      selectedEdge = null;
    } else if ((key === "Delete" || key === "Backspace") && !readonly) {
      event.preventDefault();
      remove();
    } else if (key.startsWith(t("ui.workflowCanvas.arrow")) && selected && !readonly) {
      event.preventDefault();
      const step = spec.steps.find((s) => s.id === selected);
      if (!step) return;
      const dx = key === "ArrowLeft" ? -GRID : key === "ArrowRight" ? GRID : 0;
      const dy = key === "ArrowUp" ? -GRID : key === "ArrowDown" ? GRID : 0;
      onchange?.(moveStep(spec, step.id, { x: step.x + dx, y: step.y + dy }));
    } else if (key === "+" || key === "=") {
      zoomBy(1.1);
    } else if (key === "-") {
      zoomBy(1 / 1.1);
    } else if (key === "0") {
      refit();
    } else if (key === "Escape") {
      selected = null;
      selectedEdge = null;
      menu = false;
    }
  }

  const zoomBy = (factor: number) => (view = zoomAt(view, view.zoom * factor, { x: width / 2, y: height / 2 }));
  const refit = () => (view = fit(spec.steps, { w: width, h: height }));

  function add(kind: (typeof KINDS)[number]["kind"]) {
    menu = false;
    const added = addStep(spec, kind, known, selected ?? undefined);
    if ("error" in added) return onerror?.(added.error);
    onchange?.(added.spec);
    selected = added.id;
  }

  /** Dropping a kind from the palette puts the step where it was dropped. */
  function ondrop(event: DragEvent) {
    event.preventDefault();
    const kind = event.dataTransfer?.getData("application/x-zorix-step");
    if (!kind || readonly) return;
    const box = host?.getBoundingClientRect();
    const x = (event.clientX - (box?.left ?? 0) - view.panX) / view.zoom - NODE_W / 2;
    const y = (event.clientY - (box?.top ?? 0) - view.panY) / view.zoom - NODE_H / 2;
    const added = addStep(spec, kind as (typeof KINDS)[number]["kind"], known, undefined, { x, y });
    if ("error" in added) return onerror?.(added.error);
    onchange?.(added.spec);
    selected = added.id;
  }

  const mini = $derived(miniMap(spec.steps, view, { w: width, h: height }, { w: 150, h: 96 }));
  const percent = $derived(Math.round(view.zoom * 100));
  const outline = $derived(
    spec.steps.map((s) => ({
      id: s.id,
      text: `${s.title}, ${kindLine(s)}`,
      to: spec.edges.filter((e) => e.from === s.id).map((e) => `${spec.steps.find((t) => t.id === e.to)?.title ?? e.to}${e.label ? ` (${e.label})` : ""}`),
    })),
  );

  const EDGE_STROKE: Record<string, string> = { idle: "stroke-line-strong", live: "stroke-accent", done: "stroke-ok/60", skip: "stroke-line" };
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions, a11y_no_noninteractive_tabindex -- the canvas is one composite widget: it takes the pointer and the keyboard, and lists its steps as text for screen readers -->
<div
  bind:this={host}
  bind:clientWidth={width}
  bind:clientHeight={height}
  role="application"
  aria-label={t("ui.workflowCanvas.workflowCanvasStepsArrow", { length: spec.steps.length })}
  tabindex="0"
  class={cn("relative h-full min-h-80 touch-none overflow-hidden border border-line bg-panel-2 outline-none select-none", klass)}
  style:background-image="radial-gradient(circle at 1px 1px, color-mix(in srgb, var(--faint) 45%, transparent) 1px, transparent 0)"
  style:background-size="{16 * view.zoom}px {16 * view.zoom}px"
  style:background-position="{view.panX}px {view.panY}px"
  {onpointerdown}
  {onpointermove}
  {onpointerup}
  onpointercancel={onpointerup}
  {onwheel}
  {onkeydown}
  ondragover={(e) => e.preventDefault()}
  {ondrop}
>
  <div class="absolute top-0 left-0 origin-top-left" style:transform="translate({view.panX}px, {view.panY}px) scale({view.zoom})">
    <svg class="pointer-events-none absolute top-0 left-0 overflow-visible" width="1" height="1" aria-hidden="true">
      {#each spec.edges as edge (edge.from + ">" + edge.to)}
        {@const a = place(edge.from)}
        {@const b = place(edge.to)}
        {@const state = run ? edgeState(run, edge) : "idle"}
        {@const mid = edgeMid(a, b)}
        {@const on = selectedEdge?.from === edge.from && selectedEdge?.to === edge.to}
        <path d={edgePath(a, b)} fill="none" stroke-width={on ? 2.5 : 1.5} class={cn(EDGE_STROKE[on ? "live" : state], state === "skip" && "edge-skip", state === "live" && "edge-live")} />
        {#if !readonly}
          <path data-edge="{edge.from}>{edge.to}" d={edgePath(a, b)} fill="none" stroke-width="14" class="cursor-pointer stroke-transparent" style:pointer-events="stroke" />
        {/if}
        {#if edge.label}
          <g>
            <rect x={mid.x - 15} y={mid.y - 9} width="30" height="18" rx="9" class="fill-panel stroke-line" />
            <text x={mid.x} y={mid.y + 3.5} text-anchor="middle" class="fill-muted text-2xs font-medium">{edge.label}</text>
          </g>
        {/if}
      {/each}
      {#if link}
        {@const a = place(link.from)}
        <path d={edgePath(a, { x: link.x - NODE_W / 2, y: link.y })} fill="none" stroke-width="1.5" class="edge-live stroke-accent" />
      {/if}
    </svg>

    {#each spec.steps as step (step.id)}
      {@const info = kindInfo(step.kind.type)}
      {@const at = place(step.id)}
      {@const record = run?.steps[step.id]}
      <WorkflowNode
        id={step.id}
        cls={info.cls}
        icon={info.icon}
        title={step.title}
        kind={kindLine(step)}
        detail={detailLine(step)}
        state={run ? nodeState(record?.state) : "idle"}
        selected={selected === step.id}
        badge={run ? runBadge(step.id) : undefined}
        x={at.x}
        y={at.y}
        {readonly}
        connecting={link !== null && link.from !== step.id}
      />
    {/each}
  </div>

  <ol class="sr-only" aria-label={t("ui.workflowCanvas.steps")}>
    {#each outline as row (row.id)}
      <li>{row.text}{row.to.length ? t("ui.workflowCanvas.leadsTo", { value: row.to.join(", ") }) : "."}</li>
    {/each}
  </ol>

  <div data-chrome class="glass absolute bottom-4 left-4 flex items-center gap-0.5 rounded-xl border border-glass-line p-1 shadow-raised">
    {#if !readonly}
      <div class="relative">
        <IconButton icon="plus" label={t("ui.workflowCanvas.addAStep")} size="sm" aria-haspopup="menu" aria-expanded={menu} onclick={() => (menu = !menu)} />
        {#if menu}
          <div role="menu" aria-label={t("ui.workflowCanvas.addAStep")} class="absolute bottom-10 left-0 z-20 max-h-96 w-52 overflow-y-auto rounded-xl border border-line bg-elevated p-1.5 shadow-pop">
            {#each PALETTE_GROUPS as group (group)}
              <p class="px-2 pt-1.5 pb-1 text-2xs font-semibold tracking-wider text-faint uppercase">{group}</p>
              {#each KINDS.filter((k) => k.group === group) as k (k.kind)}
                <button type="button" role="menuitem" class="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-sm hover:bg-hover" onclick={() => add(k.kind)}>
                  <Icon name={k.icon} size={14} class="text-muted" />{k.label}
                </button>
              {/each}
            {/each}
          </div>
        {/if}
      </div>
    {/if}
    <IconButton icon="zoom-in" label={t("ui.workflowCanvas.zoomIn")} size="sm" onclick={() => zoomBy(1.1)} />
    <IconButton icon="zoom-out" label={t("ui.workflowCanvas.zoomOut")} size="sm" onclick={() => zoomBy(1 / 1.1)} />
    <IconButton icon="maximize" label={t("ui.workflowCanvas.fitEverything")} size="sm" onclick={refit} />
    {#if !readonly}<IconButton icon="undo" label={t("ui.workflowCanvas.undo")} size="sm" disabled={!canUndo} onclick={() => onundo?.()} />{/if}
    <span class="tnum px-2 text-xs text-faint" aria-live="polite">{percent}%</span>
  </div>

  <div data-chrome class="glass pointer-events-none absolute right-4 bottom-4 rounded-xl border border-glass-line p-0 shadow-raised" style:width="150px" style:height="96px" aria-hidden="true">
    {#each mini.nodes as node, i (i)}
      <i class="absolute h-2 rounded-sm bg-line-strong" style:left="{node.x}px" style:top="{node.y}px" style:width="{node.w}px"></i>
    {/each}
    <b class="absolute rounded-md border-[1.5px] border-accent bg-accent-soft" style:left="{mini.viewport.x}px" style:top="{mini.viewport.y}px" style:width="{mini.viewport.w}px" style:height="{mini.viewport.h}px"></b>
  </div>
</div>

<style>
  .edge-live {
    stroke-dasharray: 5 5;
    animation: edge-flow 1s linear infinite;
  }
  .edge-skip {
    stroke-dasharray: 3 4;
  }
  @keyframes edge-flow {
    to {
      stroke-dashoffset: -10;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .edge-live {
      animation: none;
    }
  }
</style>
