<script lang="ts">
  import type { ExecutionState } from "../api.ts";
  import { stateLabel, stateTone } from "../format.ts";
  import { Badge } from "../ui/index.ts";

  let { state, compact = false }: { state: ExecutionState; compact?: boolean } = $props();

  const live = $derived(["running", "waiting_for_model", "waiting_for_tool", "waiting_for_children", "queued"].includes(state));
</script>

{#if compact}
  <span title={stateLabel(state)}><Badge tone={stateTone(state)} {live} dot>{""}</Badge></span>
{:else}
  <Badge tone={stateTone(state)} {live} dot>{stateLabel(state)}</Badge>
{/if}
