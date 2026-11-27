<script lang="ts">
  import { t } from "../../i18n/index.ts";
  import { errorText } from "../../api.ts";
  import { settingsStore } from "../../stores/appSettings.svelte.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { Card, ToggleRow } from "../../ui/index.ts";

  const on = $derived(settingsStore.value?.context.summaries ?? true);

  async function change(next: boolean) {
    try {
      await settingsStore.save((s) => ({ ...s, context: { ...s.context, summaries: next } }));
      toast.ok(next ? t("settings.summariesAreOn") : t("settings.summariesAreOff"));
    } catch (e) {
      toast.err(errorText(e));
      await settingsStore.load();
    }
  }
</script>

<Card title={t("settings.longConversations")} icon="message">
  <ToggleRow
    checked={on}
    label={t("settings.summariseWhatNoLongerFits")}
    description={t("settings.whenAConversationOutgrows")}
    disabled={settingsStore.value === null}
    onchange={(next) => void change(next)}
  />
</Card>
