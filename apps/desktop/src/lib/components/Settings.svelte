<script lang="ts">
  import { t } from "../i18n/index.ts";
  import { type Mode, MODE_LABEL } from "../theme.ts";
  import { store } from "../stores/runs.svelte.ts";
  import { theme } from "../stores/theme.svelte.ts";
  import { type HostInfo, errorText, workflowHost, workflowSettingsSet } from "../api.ts";
  import { settingsStore } from "../stores/appSettings.svelte.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import KeepRuns from "./settings/KeepRuns.svelte";
  import Telemetry from "./settings/Telemetry.svelte";
  import OtherTools from "./settings/OtherTools.svelte";
  import LongConversations from "./settings/LongConversations.svelte";
  import { BrandMark, Button, Callout, Card, Field, Input, KeyValue, PageHeader, Segmented, Tabs, ToggleRow } from "../ui/index.ts";

  let tab = $state("general");
  let mode = $state<Mode>(theme.mode);
  let host = $state<HostInfo | null>(null);
  let webhooks = $state(false);
  let address = $state("");
  let saving = $state(false);
  $effect(() => {
    if (tab !== "general") return;
    void settingsStore.load();
    void workflowHost().then((h) => {
      host = h;
      webhooks = h.webhooks;
      address = h.address;
    });
  });
  const changed = $derived(host !== null && (webhooks !== host.webhooks || address.trim() !== host.address));

  async function saveWebhooks() {
    saving = true;
    try {
      host = await workflowSettingsSet(webhooks, address);
      webhooks = host.webhooks;
      address = host.address;
      toast.ok(host.webhooks ? t("settings.webhooksArriveOn", { address: host.listening ?? host.address }) : t("settings.webhooksAreOff"));
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      saving = false;
    }
  }
  $effect(() => {
    if (mode !== theme.mode) theme.set(mode);
  });
</script>

<div class="mx-auto flex h-full max-w-4xl flex-col gap-5 overflow-y-auto p-6 [&>*]:shrink-0">
  <PageHeader title={t("common.settings")} subtitle={t("settings.howZorixLooksAnd")} glow class="mb-1" />

  <Tabs
    tabs={[
      { value: "general", label: "General" },
      { value: "appearance", label: "Appearance" },
      { value: "about", label: "About" },
    ]}
    bind:value={tab}
    label={t("common.settings")}
  />

  {#if tab === "general"}
    <Card title={t("settings.data")} icon="database">
      <KeyValue
        items={[
          { key: "Data folder", value: store.info?.data_dir, mono: true },
          { key: "Version", value: store.info?.version, mono: true },
        ]}
      />
      <p class="mt-3 text-sm text-muted">{t("settings.runsMemoryAgentsAnd")}</p>
    </Card>
    <LongConversations />
    <KeepRuns />
    <Telemetry />
    <OtherTools />
    <Card title={t("settings.webhooks")} icon="webhook">
      <p class="mb-3 text-base text-muted">{t("settings.aWorkflowCanStart")} <span class="mono">{t("settings.zorixServe")}</span>.</p>
      <ToggleRow label={t("settings.receiveWebhooks")} description={webhooks ? t("settings.zorixListensForRequests") : t("settings.nothingListens")} bind:checked={webhooks} />
      {#if webhooks}
        <div class="mt-3 max-w-sm">
          <Field label={t("common.address")} hint={t("settings.127001")}>
            {#snippet children(f)}<Input id={f.id} mono bind:value={address} placeholder="127.0.0.1:7788" />{/snippet}
          </Field>
        </div>
      {/if}
      {#if host && !host.running}<div class="mt-3"><Callout tone="warn" title={t("settings.theWorkflowHostIs")}>{t("settings.schedulesAndWebhooksAre")}</Callout></div>{/if}
      <div class="mt-4 flex items-center gap-3">
        <Button variant="primary" disabled={!changed || saving} onclick={() => void saveWebhooks()}>{t("common.save")}</Button>
        {#if host?.listening}<span class="mono text-xs text-muted">{t("settings.listeningOn", { listening: host.listening })}</span>{/if}
      </div>
    </Card>
  {:else if tab === "appearance"}
    <Card title={t("settings.theme")} icon="sun">
      <p class="mb-3 text-base text-muted">{t("settings.lightIsTheDefault")}</p>
      <Segmented
        label={t("settings.theme")}
        bind:value={mode}
        options={[
          { value: "light", label: MODE_LABEL.light, icon: "sun" },
          { value: "dark", label: MODE_LABEL.dark, icon: "moon" },
          { value: "system", label: MODE_LABEL.system, icon: "monitor" },
        ]}
      />
    </Card>
  {:else}
    <Card>
      <div class="flex flex-col items-center gap-3 px-4 py-8 text-center">
        <BrandMark size="lg" />
        <p class="text-2xl font-semibold tracking-tight">ZORIX</p>
        <p class="max-w-[46ch] text-base text-muted">{t("settings.aHarnessForAi")}</p>
        <p class="mono text-sm text-muted">{t("settings.version", { value: store.info?.version ?? "" })}</p>
        <p class="text-xs text-muted">{t("settings.mitLicenceFontsAnd")}</p>
      </div>
    </Card>
  {/if}
</div>
