<script lang="ts">
  import { onDestroy, onMount } from "svelte";

  import { t } from "../../i18n/index.ts";
  import { type OcrStatus, errorText, ocrSet, ocrStatus, onOcrProgress } from "../../api.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { Badge, Callout, Card, Icon, ToggleRow } from "../../ui/index.ts";

  /** Called after the setting changed, so the folders can show what was read. */
  let { onchange }: { onchange?: () => void } = $props();

  let status = $state<OcrStatus | null>(null);
  let enabled = $state(false);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let stop: (() => void) | null = null;

  const percent = $derived(status && status.total > 0 ? Math.min(100, Math.round((status.done / status.total) * 100)) : 0);

  async function load() {
    try {
      status = await ocrStatus();
      enabled = status.enabled;
    } catch (e) {
      error = errorText(e);
    }
  }

  onMount(async () => {
    await load();
    stop = await onOcrProgress((p) => {
      if (status) status = { ...status, state: "installing", done: p.done, total: p.total };
    });
  });
  onDestroy(() => stop?.());

  async function change(next: boolean) {
    busy = true;
    error = null;
    if (next && status) status = { ...status, state: "installing", done: 0 };
    try {
      status = await ocrSet(next);
      enabled = status.enabled;
      toast.ok(next ? t("knowledge.readingScansIsOn") : t("knowledge.readingScansIsOff"));
      onchange?.();
    } catch (e) {
      error = errorText(e);
      await load();
    } finally {
      busy = false;
    }
  }
</script>

<Card title={t("knowledge.readingScans")} icon="file-text">
  <ToggleRow
    bind:checked={enabled}
    class="!px-0"
    label={t("knowledge.readTheTextOf")}
    description={t("knowledge.usesTwoSmallModels")}
    disabled={status === null || busy}
    onchange={(next) => void change(next)}
  />
  <div class="mt-1.5 flex flex-wrap items-center gap-2">
    {#if status?.state === "ready"}
      <Badge tone="ok"><Icon name="check" size={12} />{t("knowledge.modelsReady")}</Badge>
      <span class="text-xs text-faint">{t("knowledge.looksForEnglishAnd")}</span>
    {:else if status?.state === "installing"}
      <div class="w-full" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow={percent} aria-label={t("knowledge.downloadingTheModels", { percent })}>
        <div class="h-1.5 overflow-hidden rounded-full bg-hover"><div class="brand-gradient h-full rounded-full transition-all duration-200" style:width="{percent}%"></div></div>
        <p class="mt-1.5 text-xs text-muted">{t("knowledge.downloadingTheModels", { percent })}</p>
      </div>
    {:else if status?.state === "missing"}
      <Badge tone="warn" dot>{t("knowledge.modelsMissing")}</Badge>
      <span class="text-xs text-muted">{t("knowledge.theModelsAreNot")}</span>
    {/if}
  </div>
  {#if error}<Callout tone="err" class="mt-3">{error}</Callout>{/if}
</Card>
