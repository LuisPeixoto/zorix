<script lang="ts">
  import { t } from "../../i18n/index.ts";
  import { errorText, telemetrySetHeader } from "../../api.ts";
  import { settingsStore } from "../../stores/appSettings.svelte.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { Button, Callout, Card, Field, Input } from "../../ui/index.ts";

  let endpoint = $state("");
  let headerName = $state("");
  // Never filled from what is saved: the value lives in the system keychain and is not sent back. Typing one replaces it.
  let headerValue = $state("");
  let loaded = $state(false);
  let saving = $state(false);

  $effect(() => {
    const saved = settingsStore.value;
    if (saved && !loaded) {
      endpoint = saved.telemetry.endpoint ?? "";
      headerName = saved.telemetry.header ?? "";
      loaded = true;
    }
  });

  const savedEndpoint = $derived(settingsStore.value?.telemetry.endpoint ?? "");
  const savedHeader = $derived(settingsStore.value?.telemetry.header ?? "");
  const on = $derived(savedEndpoint !== "");
  const changed = $derived(endpoint.trim() !== savedEndpoint || headerName.trim() !== savedHeader || headerValue !== "");
  const headerIncomplete = $derived(headerName.trim() !== "" && headerValue === "" && savedHeader === "");

  async function save() {
    saving = true;
    try {
      // The header first: it is kept in the keychain, and the address then starts the export with it.
      if (headerName.trim() === "" || headerValue !== "") {
        await telemetrySetHeader(headerName.trim() === "" ? null : headerName.trim(), headerValue === "" ? null : headerValue);
        headerValue = "";
      }
      await settingsStore.save((s) => ({ ...s, telemetry: { ...s.telemetry, endpoint: endpoint.trim() === "" ? null : endpoint.trim() } }));
      toast.ok(endpoint.trim() === "" ? t("settings.telemetryIsOff") : t("settings.telemetryIsOn"));
    } catch (e) {
      toast.err(errorText(e));
      await settingsStore.load();
    } finally {
      saving = false;
    }
  }

  async function turnOff() {
    endpoint = "";
    headerName = "";
    headerValue = "";
    await save();
  }
</script>

<Card title={t("settings.telemetry")} icon="activity">
  <p class="mb-3 text-base text-muted">{t("settings.sendWhatHappensInA")}</p>
  <div class="grid items-start gap-4" style:grid-template-columns="minmax(0, 1fr) 16.25rem">
    <Field label={t("settings.collectorAddress")} hint={t("settings.httpsOrHttpOn")}>
      {#snippet children(f)}<Input id={f.id} mono bind:value={endpoint} placeholder="https://otel.example.com:4318" />{/snippet}
    </Field>
    <Field label={t("settings.header")} hint={savedHeader ? t("settings.keptInTheSystemKeychain") : t("settings.forACollectorThatAsks")}>
      {#snippet children(f)}<Input id={f.id} mono bind:value={headerName} placeholder="authorization" />{/snippet}
    </Field>
  </div>
  {#if headerName.trim() !== ""}
    <div class="mt-3 max-w-md">
      <Field label={t("settings.headerValue")} hint={savedHeader ? t("settings.leaveEmptyToKeepThe") : undefined}>
        {#snippet children(f)}<Input id={f.id} mono type="password" autocomplete="off" bind:value={headerValue} placeholder={savedHeader ? "••••••••" : t("settings.bearerPlaceholder")} />{/snippet}
      </Field>
    </div>
  {/if}
  <div class="mt-4">
    <Callout tone="warn" icon="shield" title={t("settings.thisLeavesYourComputer")}>{t("settings.eachSpanCarriesTheModel")}</Callout>
  </div>
  <div class="mt-4 flex items-center justify-end gap-2">
    {#if on}<Button disabled={saving} onclick={() => void turnOff()}>{t("settings.turnOff")}</Button>{/if}
    <Button variant="primary" disabled={!changed || headerIncomplete || saving} loading={saving} onclick={() => void save()}>{t("common.save")}</Button>
  </div>
</Card>
