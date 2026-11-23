<script lang="ts">
  import { t } from "../i18n/index.ts";
  import Badge from "./Badge.svelte";
  import type { UiTone } from "./tone.ts";

  /** What a connection or server is doing, in one consistent look. A live pulse means something is happening now. */
  export type PillState = "ready" | "starting" | "restarting" | "needs_login" | "failed" | "stopped" | "off" | "review";

  let { state, label }: { state: PillState; label?: string } = $props();

  const MAP: Record<PillState, { tone: UiTone; text: string; live: boolean }> = {
    ready: { tone: "ok", text: t("ui.statusPill.ready"), live: false },
    starting: { tone: "info", text: t("ui.statusPill.starting"), live: true },
    restarting: { tone: "warn", text: t("ui.statusPill.restarting"), live: true },
    needs_login: { tone: "warn", text: t("ui.statusPill.signIn"), live: false },
    failed: { tone: "err", text: t("common.failed"), live: false },
    stopped: { tone: "muted", text: t("common.stopped"), live: false },
    off: { tone: "muted", text: t("ui.statusPill.off"), live: false },
    review: { tone: "accent", text: t("common.review"), live: false },
  };
  const shown = $derived(MAP[state]);
</script>

<Badge tone={shown.tone} dot live={shown.live}>{label ?? shown.text}</Badge>
