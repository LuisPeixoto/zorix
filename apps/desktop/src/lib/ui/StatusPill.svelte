<script lang="ts">
  import Badge from "./Badge.svelte";
  import type { UiTone } from "./tone.ts";

  /** What a connection or server is doing, in one consistent look. A live pulse means something is happening now. */
  export type PillState = "ready" | "starting" | "restarting" | "needs_login" | "failed" | "stopped" | "off" | "review";

  let { state, label }: { state: PillState; label?: string } = $props();

  const MAP: Record<PillState, { tone: UiTone; text: string; live: boolean }> = {
    ready: { tone: "ok", text: "Ready", live: false },
    starting: { tone: "info", text: "Starting", live: true },
    restarting: { tone: "warn", text: "Restarting", live: true },
    needs_login: { tone: "warn", text: "Sign in", live: false },
    failed: { tone: "err", text: "Failed", live: false },
    stopped: { tone: "muted", text: "Stopped", live: false },
    off: { tone: "muted", text: "Off", live: false },
    review: { tone: "accent", text: "Review", live: false },
  };
  const shown = $derived(MAP[state]);
</script>

<Badge tone={shown.tone} dot live={shown.live}>{label ?? shown.text}</Badge>
