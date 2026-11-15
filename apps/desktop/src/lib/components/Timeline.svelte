<script lang="ts">
  import type { ExecEvent } from "../api.ts";
  import { describeEvent, isNoise } from "../format.ts";

  let { events }: { events: ExecEvent[] } = $props();

  const shown = $derived(events.filter((e) => !isNoise(e)));
  const dot = { ok: "bg-ok", warn: "bg-warn", err: "bg-err", info: "bg-info", muted: "bg-faint" } as const;

  const time = (at: number) => new Date(at).toLocaleTimeString([], { hour12: false });
</script>

<ol class="space-y-0.5" aria-label="What happened">
  {#each shown as event (event.seq)}
    {@const view = describeEvent(event)}
    <li class="flex items-baseline gap-3 rounded-md px-2 py-1 hover:bg-hover">
      <span class="mt-1 size-2 shrink-0 self-start rounded-full {dot[view.tone]}"></span>
      <span class="w-16 shrink-0 font-mono text-xs text-faint">{time(event.at)}</span>
      <span class="shrink-0 font-medium">{view.title}</span>
      {#if view.detail}<span class="min-w-0 truncate text-muted" title={view.detail}>{view.detail}</span>{/if}
    </li>
  {/each}
  {#if shown.length === 0}
    <li class="px-2 py-1 text-muted">Nothing yet.</li>
  {/if}
</ol>
