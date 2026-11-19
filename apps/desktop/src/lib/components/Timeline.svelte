<script lang="ts">
  import type { ExecEvent } from "../api.ts";
  import { describeEvent, eventIcon, isNoise } from "../format.ts";
  import { Badge, Icon, toneClass } from "../ui/index.ts";

  let { events }: { events: ExecEvent[] } = $props();

  const shown = $derived(events.filter((e) => !isNoise(e)));
  const time = (at: number) => new Date(at).toLocaleTimeString([], { hour12: false });
</script>

<ol aria-label="What happened" class="relative">
  {#each shown as event, i (event.seq)}
    {@const view = describeEvent(event)}
    <li class="relative flex gap-3 pb-3 last:pb-0">
      {#if i < shown.length - 1}<span class="absolute top-7 bottom-0 left-[13px] w-px bg-line" aria-hidden="true"></span>{/if}
      <span class="relative z-10 grid size-7 shrink-0 place-items-center rounded-full {toneClass(view.tone, 'soft')}">
        <Icon name={eventIcon(event.kind)} size={14} />
      </span>
      <div class="min-w-0 flex-1 pt-0.5">
        <div class="flex items-baseline gap-2">
          <span class="text-md font-medium">{view.title}</span>
          {#if event.kind.includes(".tool.") && typeof event.payload.source === "string" && event.payload.source !== "builtin"}
            <Badge tone={event.payload.source.startsWith("mcp:") ? "accent" : "muted"} class="h-5">{event.payload.source}</Badge>
          {/if}
          <span class="mono ml-auto shrink-0 text-xs text-faint tabular-nums">{time(event.at)}</span>
        </div>
        {#if view.detail}<p class="selectable text-base break-words text-muted">{view.detail}</p>{/if}
      </div>
    </li>
  {/each}
  {#if shown.length === 0}
    <li class="text-base text-muted">Nothing yet.</li>
  {/if}
</ol>
