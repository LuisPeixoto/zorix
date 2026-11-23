<script lang="ts" generics="T extends { id: string; lane: string; label: string }">
  import { t } from "../i18n/index.ts";
  import { type Snippet, tick } from "svelte";

  import { dropIndex, nextSpot } from "../project.ts";
  import Button from "./Button.svelte";
  import Dot from "./Dot.svelte";
  import type { UiTone } from "./tone.ts";

  /**
   * A project's tasks in lanes (design system: TaskBoard). A card is dragged with the pointer, which tilts it and lifts it, or with
   * the keyboard: Space picks it up, the arrows move it between lanes and places, Space drops it and Escape lets it go. Every step
   * is said in a live region. The board only says where a card was dropped; what that means is decided elsewhere.
   */
  let {
    lanes,
    items,
    card,
    onmove,
    onadd,
    onopen,
    moving = null,
  }: {
    lanes: { id: string; name: string; tone: UiTone }[];
    /** In display order within each lane. */
    items: T[];
    /** The card. `open` opens the item unless it was just dragged. */
    card: Snippet<[item: T, dragging: boolean, open: () => void]>;
    onmove: (item: T, lane: string, index: number) => void;
    onadd?: (lane: string) => void;
    /** A move is being saved: cards stay where they are until it is. */
    moving?: string | null;
    onopen: (item: T) => void;
  } = $props();

  type Spot = { lane: string; index: number };
  let drag = $state<{ id: string; pointer: number; x: number; y: number; dx: number; dy: number; active: boolean } | null>(null);
  let picked = $state<string | null>(null);
  let spot = $state<Spot | null>(null);
  let announcement = $state("");
  let refocus = $state<string | null>(null);
  let justDragged = false;
  const laneEls: Record<string, HTMLElement | undefined> = {};

  const lists = $derived.by(() => {
    const out: Record<string, T[]> = Object.fromEntries(lanes.map((l) => [l.id, [] as T[]]));
    for (const item of items) out[item.lane]?.push(item);
    return out;
  });
  const movingId = $derived(drag?.active ? drag.id : picked);

  /** How many other cards each lane has, without the one that is moving. */
  const others = $derived(Object.fromEntries(lanes.map((l) => [l.id, (lists[l.id] ?? []).filter((i) => i.id !== movingId).length])));

  function rows(lane: string): ({ item: T } | { gap: true })[] {
    const out: ({ item: T } | { gap: true })[] = [];
    let place = 0;
    for (const item of lists[lane] ?? []) {
      if (item.id !== movingId) {
        if (spot && spot.lane === lane && spot.index === place) out.push({ gap: true });
        place += 1;
      }
      out.push({ item });
    }
    if (spot && spot.lane === lane && spot.index === place) out.push({ gap: true });
    return out;
  }

  function positionOf(item: T): Spot {
    const list = (lists[item.lane] ?? []).filter((i) => i.id !== item.id);
    const before = (lists[item.lane] ?? []).findIndex((i) => i.id === item.id);
    return { lane: item.lane, index: Math.min(before, list.length) };
  }

  const laneName = (id: string) => lanes.find((l) => l.id === id)?.name ?? id;

  function pointerDown(event: PointerEvent, item: T) {
    if (event.button !== 0 || picked || moving) return;
    drag = { id: item.id, pointer: event.pointerId, x: event.clientX, y: event.clientY, dx: 0, dy: 0, active: false };
  }

  function pointerMove(event: PointerEvent, item: T) {
    if (!drag || drag.pointer !== event.pointerId || drag.id !== item.id) return;
    const dx = event.clientX - drag.x;
    const dy = event.clientY - drag.y;
    if (!drag.active) {
      if (Math.hypot(dx, dy) < 6) return;
      (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
      drag.active = true;
      spot = positionOf(item);
    }
    drag.dx = dx;
    drag.dy = dy;
    // The lane under the pointer (or the nearest one sideways), and the place among its other cards.
    let target: string | null = null;
    let best = Number.POSITIVE_INFINITY;
    for (const lane of lanes) {
      const box = laneEls[lane.id]?.getBoundingClientRect();
      if (!box) continue;
      const gap = event.clientX < box.left ? box.left - event.clientX : event.clientX > box.right ? event.clientX - box.right : 0;
      if (gap < best) {
        best = gap;
        target = lane.id;
      }
    }
    if (!target) return;
    const middles = (lists[target] ?? [])
      .filter((i) => i.id !== item.id)
      .map((i) => {
        const box = laneEls[target as string]?.querySelector(`[data-id="${CSS.escape(i.id)}"]`)?.getBoundingClientRect();
        return box ? box.top + box.height / 2 : 0;
      });
    spot = { lane: target, index: dropIndex(middles, event.clientY) };
  }

  function pointerUp(event: PointerEvent, item: T) {
    if (!drag || drag.pointer !== event.pointerId || drag.id !== item.id) return;
    const was = drag;
    const target = spot;
    drag = null;
    spot = null;
    if (!was.active) return;
    justDragged = true;
    setTimeout(() => (justDragged = false), 0);
    if (!target) return;
    const here = positionOf(item);
    if (target.lane !== here.lane || target.index !== here.index) {
      refocus = item.id;
      onmove(item, target.lane, target.index);
    }
  }

  function pointerCancel() {
    drag = null;
    spot = null;
  }

  function keyDown(event: KeyboardEvent, item: T) {
    if (event.target !== event.currentTarget && !(event.target as HTMLElement).closest("button")) return;
    if (picked === null) {
      if (event.key === " " && !moving) {
        event.preventDefault();
        picked = item.id;
        spot = positionOf(item);
        announcement = t("ui.taskBoard.pickedUpTheArrow", { label: item.label });
      }
      return;
    }
    if (picked !== item.id || !spot) return;
    if (event.key === "Escape") {
      event.preventDefault();
      picked = null;
      spot = null;
      announcement = t("ui.taskBoard.staysWhereItWas", { label: item.label });
      return;
    }
    if (event.key === " " || event.key === "Enter") {
      event.preventDefault();
      const target = spot;
      picked = null;
      spot = null;
      announcement = t("ui.taskBoard.droppedInPlace", { label: item.label, value: laneName(target.lane), value2: target.index + 1 });
      refocus = item.id;
      onmove(item, target.lane, target.index);
      return;
    }
    if ([t("ui.taskBoard.arrowleft"), t("ui.taskBoard.arrowright"), t("ui.taskBoard.arrowup"), t("ui.taskBoard.arrowdown")].includes(event.key)) {
      event.preventDefault();
      const next = nextSpot(lanes.map((l) => l.id), others, spot, event.key);
      spot = next;
      announcement = t("ui.taskBoard.placeOf", { value: laneName(next.lane), value2: next.index + 1, value3: (others[next.lane] ?? 0) + 1 });
    }
  }

  // A card that was moved keeps the focus, so a person using the keyboard can move it again.
  $effect(() => {
    void items;
    const id = refocus;
    if (!id) return;
    void tick().then(() => {
      for (const el of Object.values(laneEls)) {
        const button = el?.querySelector<HTMLElement>(t("ui.taskBoard.dataIdButton", { value: CSS.escape(id) }));
        if (button) {
          button.focus();
          refocus = null;
          return;
        }
      }
    });
  });
</script>

<div class="grid gap-3 pb-1" style:grid-template-columns="repeat({lanes.length}, minmax(200px, 1fr))">
  {#each lanes as lane (lane.id)}
    {@const count = lists[lane.id]?.length ?? 0}
    <section bind:this={laneEls[lane.id]} aria-label="{lane.name}, {count} {count === 1 ? 'task' : 'tasks'}" class="flex min-w-0 flex-col gap-2 self-start rounded-2xl border border-line-soft bg-panel-2 p-2.5">
      <header class="flex items-center gap-2 px-1 pt-0.5 pb-1 text-sm font-semibold">
        <Dot tone={lane.tone} />
        {lane.name}
        <span class="tnum ml-auto rounded-full bg-hover px-1.5 text-2xs font-medium text-muted">{count}</span>
      </header>
      <ul class="flex flex-col gap-2" role="list">
        {#each rows(lane.id) as row, i (("item" in row ? row.item.id : `gap-${i}`))}
          {#if "gap" in row}
            <li aria-hidden="true" class="h-14 rounded-xl border border-dashed border-accent/60 bg-accent-soft/50"></li>
          {:else}
            {@const dragging = movingId === row.item.id}
            <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
            <!-- The card inside is the button: the pointer drags the item around it, and the keys arrive from that button. -->
            <li
              data-id={row.item.id}
              class={dragging && drag?.active ? "relative z-30 touch-none" : "touch-none"}
              style:transform={dragging && drag?.active ? `translate(${drag.dx}px, ${drag.dy}px)` : undefined}
              onpointerdown={(e) => pointerDown(e, row.item)}
              onpointermove={(e) => pointerMove(e, row.item)}
              onpointerup={(e) => pointerUp(e, row.item)}
              onpointercancel={pointerCancel}
              onkeydown={(e) => keyDown(e, row.item)}
            >
              {@render card(row.item, dragging, () => { if (!justDragged && picked === null) onopen(row.item); })}
            </li>
          {/if}
        {/each}
      </ul>
      {#if onadd}
        <Button variant="ghost" size="sm" icon="plus" class="justify-start" onclick={() => onadd(lane.id)}>{t("common.addTask")}</Button>
      {/if}
    </section>
  {/each}
</div>
<p class="sr-only" role="status" aria-live="polite">{announcement}</p>
