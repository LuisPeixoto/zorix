<script lang="ts">
  import type { ExecutionState } from "../api.ts";
  import { stateLabel, stateTone } from "../format.ts";

  let { state, compact = false }: { state: ExecutionState; compact?: boolean } = $props();

  const tone = $derived(stateTone(state));
  const colour = $derived(
    { ok: "text-ok", warn: "text-warn", err: "text-err", info: "text-info", muted: "text-muted" }[tone],
  );
  const pulsing = $derived(["running", "waiting_for_model", "waiting_for_tool", "queued"].includes(state));
</script>

<span class="inline-flex items-center gap-1.5 font-medium {colour}" title={stateLabel(state)}>
  <span class="size-2 shrink-0 rounded-full bg-current {pulsing ? 'animate-pulse' : ''}"></span>
  {#if !compact}{stateLabel(state)}{/if}
</span>
