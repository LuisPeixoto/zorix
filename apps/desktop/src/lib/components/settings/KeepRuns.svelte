<script lang="ts">
  import { fmt, t } from "../../i18n/index.ts";
  import { type RetentionReport, errorText, retentionPreview, retentionRun } from "../../api.ts";
  import { settingsStore } from "../../stores/appSettings.svelte.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { Button, Callout, Card, Field, Input, Modal, Segmented } from "../../ui/index.ts";

  type Mode = "compact" | "delete";

  let days = $state("");
  let mode = $state<Mode>("compact");
  let loaded = $state(false);
  let preview = $state<RetentionReport | null>(null);
  let saving = $state(false);
  let running = $state(false);
  let confirm = $state(false);

  // What the form shows starts from what is saved.
  $effect(() => {
    const saved = settingsStore.value;
    if (saved && !loaded) {
      days = saved.retention.days === null ? "" : String(saved.retention.days);
      mode = saved.retention.mode;
      loaded = true;
    }
  });

  const parsed = $derived(days.trim() === "" ? null : Number(days));
  const valid = $derived(parsed === null || (Number.isInteger(parsed) && parsed >= 1 && parsed <= 3650));
  const changed = $derived(
    settingsStore.value !== null && ((parsed ?? null) !== settingsStore.value.retention.days || mode !== settingsStore.value.retention.mode),
  );
  const savedOn = $derived(settingsStore.value?.retention.days != null);

  // How many runs are old enough, for what is typed, before anything is saved or done.
  $effect(() => {
    const [d, m] = [parsed, mode];
    preview = null;
    if (d === null || !valid) return;
    const timer = setTimeout(() => {
      void retentionPreview(d, m).then((r) => (preview = r)).catch(() => (preview = null));
    }, 300);
    return () => clearTimeout(timer);
  });

  async function save() {
    saving = true;
    try {
      await settingsStore.save((s) => ({ ...s, retention: { days: parsed, mode } }));
      toast.ok(parsed === null ? t("settings.runsAreKeptForEver") : t("settings.keepingRunsFor", { days: parsed }));
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      saving = false;
    }
  }

  async function runNow() {
    confirm = false;
    running = true;
    try {
      const report = await retentionRun();
      toast.ok(t("settings.retentionDone", { count: report.runs }));
      preview = null;
      if (parsed !== null && valid) preview = await retentionPreview(parsed, mode);
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      running = false;
    }
  }

  const megabytes = (bytes: number) => fmt.number(Math.max(0.1, Math.round(bytes / 104857.6) / 10));
</script>

<Card title={t("settings.keepRuns")} icon="clock">
  <p class="mb-3 text-base text-muted">{t("settings.oldRunsTakeRoom")}</p>
  <div class="grid items-start gap-4" style:grid-template-columns="12.5rem minmax(0, 1fr)">
    <Field label={t("settings.keepFinishedRunsFor")} hint={t("settings.oneTo3650Empty")}>
      {#snippet children(f)}
        <Input id={f.id} bind:value={days} inputmode="numeric" invalid={!valid} placeholder={t("settings.forEver")}>
          {#snippet trailing()}<span class="text-sm text-muted">{t("settings.days")}</span>{/snippet}
        </Input>
      {/snippet}
    </Field>
    <Field label={t("settings.whatHappensToAnOldRun")} hint={mode === "compact" ? t("settings.compactKeepsTheRequest") : t("settings.deleteRemovesTheRun")}>
      {#snippet children()}
        <Segmented
          label={t("settings.whatHappensToAnOldRun")}
          bind:value={mode}
          options={[
            { value: "compact", label: t("settings.compact") },
            { value: "delete", label: t("settings.delete") },
          ]}
        />
      {/snippet}
    </Field>
  </div>
  {#if preview && parsed !== null}
    <div class="mt-4">
      {#if preview.runs === 0}
        <Callout tone="info" title={t("settings.nothingIsOldEnough")}>{t("settings.noFinishedRunIsOlder", { days: parsed })}</Callout>
      {:else}
        <Callout tone={mode === "delete" ? "warn" : "info"} title={t("settings.runsAreOlderThan", { count: preview.runs, days: parsed })}>
          {mode === "delete" ? t("settings.deletingThemFreesAbout", { size: megabytes(preview.bytes) }) : t("settings.compactingThemFreesAbout", { size: megabytes(preview.bytes) })}
          {#if preview.kept_because_referenced > 0}{" "}{t("settings.referredRunsAreCompacted", { count: preview.kept_because_referenced })}{/if}
        </Callout>
      {/if}
    </div>
  {/if}
  <div class="mt-4 flex items-center justify-end gap-2">
    <Button icon="trash" disabled={!savedOn || changed || running} loading={running} onclick={() => (confirm = true)}>
      {mode === "delete" ? t("settings.deleteNow") : t("settings.compactNow")}
    </Button>
    <Button variant="primary" disabled={!changed || !valid || saving} loading={saving} onclick={() => void save()}>{t("common.save")}</Button>
  </div>
</Card>

<Modal bind:open={confirm} title={mode === "delete" ? t("settings.deleteOldRuns") : t("settings.compactOldRuns")} icon="trash" size="sm">
  <p class="text-base text-muted">
    {mode === "delete" ? t("settings.thisRemovesEveryFinished", { days: settingsStore.value?.retention.days ?? 0 }) : t("settings.thisDropsTheEventsThe", { days: settingsStore.value?.retention.days ?? 0 })}
  </p>
  {#snippet footer()}
    <Button onclick={() => (confirm = false)}>{t("common.cancel")}</Button>
    <Button variant="danger" icon="trash" onclick={() => void runNow()}>{mode === "delete" ? t("settings.deleteNow") : t("settings.compactNow")}</Button>
  {/snippet}
</Modal>
