<script lang="ts">
  import { untrack } from "svelte";
  import { t } from "../../i18n/index.ts";
  import { open as pickFile } from "@tauri-apps/plugin-dialog";

  import { type ConnectionKind, type DeviceStep, type SignInInfo, connectionSignIn, connectionSignInCancel, connectionSignInInfo, connectionSignInOpen, errorText } from "../../api.ts";
  import { integrations } from "../../stores/integrations.svelte.ts";
  import { KINDS } from "./kinds.ts";
  import { BrandLogo, Button, Callout, Field, Icon, Input, Modal, Segmented, Stepper } from "../../ui/index.ts";

  let { open = $bindable(false), kind }: { open?: boolean; kind: ConnectionKind | null } = $props();

  const steps = [{ label: t("integrations.connectModal.details") }, { label: t("integrations.connectModal.secret"), hint: t("integrations.connectModal.keptInYourKeychain") }, { label: t("common.test") }];
  let step = $state(0);
  let name = $state("");
  let values = $state<Record<string, string>>({});
  let secret = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);
  let result = $state<{ ok: boolean; note: string } | null>(null);
  let added = $state(false);
  // Signing in, when the service offers it.
  let signInInfo = $state<SignInInfo | null>(null);
  let method = $state<"signin" | "token">("token");
  let clientId = $state("");
  let clientSecret = $state("");
  let device = $state<DeviceStep | null>(null);
  let waiting = $state(false);

  const spec = $derived(kind ? KINDS[kind] : null);
  const taken = $derived(integrations.connections.some((c) => c.name === name));

  $effect(() => {
    if (open) {
      step = 0;
      name = kind ?? "";
      values = {};
      secret = "";
      busy = false;
      error = null;
      result = null;
      added = false;
      signInInfo = null;
      method = "token";
      clientId = "";
      clientSecret = "";
      device = null;
      waiting = false;
      const asked = kind;
      if (asked)
        void connectionSignInInfo(asked).then((info) => {
          if (kind !== asked) return;
          signInInfo = info;
          if (info) method = "signin";
        });
    }
  });

  // Closing the window while a sign-in waits gives it up.
  $effect(() => {
    if (!open && waiting) void connectionSignInCancel(untrack(() => name.trim()));
  });

  const signing = $derived(method === "signin" && !!signInInfo);
  // The account email only matters for Jira's API token.
  const fields = $derived((spec?.fields ?? []).filter((f) => !(signing && f.key === "email")));
  const appReady = $derived(!!signInInfo && (signInInfo.builtin || !!clientId.trim()) && (!signInInfo.needs_secret || !!clientSecret.trim()));

  const ready = $derived(!!spec && !!name.trim() && !taken && fields.every((f) => !f.required || values[f.key]?.trim()));

  async function choose(key: string) {
    try {
      const file = await pickFile({ directory: false, multiple: false, title: t("integrations.connectModal.chooseTheDatabaseFile") });
      if (typeof file === "string") values[key] = file;
    } catch (e) {
      error = errorText(e);
    }
  }

  function settings(): Record<string, unknown> {
    const out: Record<string, unknown> = {};
    for (const field of fields) {
      const value = values[field.key]?.trim();
      if (!value) continue;
      out[field.key] = field.number ? Number(value) : value;
    }
    if (kind === "http" && out.header) {
      out.auth = "header";
    } else if (kind === "http") {
      out.auth = "bearer";
    }
    return out;
  }

  async function signIn() {
    if (!kind) return;
    error = null;
    device = null;
    waiting = true;
    busy = true;
    try {
      await connectionSignIn(
        { kind, name: name.trim(), settings: settings() },
        { clientId: clientId.trim() || undefined, clientSecret: clientSecret.trim() || undefined },
        (step) => (device = step),
      );
      added = true;
      clientSecret = "";
      await integrations.refresh();
      step = 2;
      result = await integrations.testConnection(name.trim());
    } catch (e) {
      if (open) error = errorText(e);
    } finally {
      waiting = false;
      busy = false;
    }
  }

  async function addAndTest() {
    if (!kind || !spec) return;
    error = null;
    busy = true;
    result = null;
    try {
      if (!added) {
        const row = await integrations.addConnection(kind, name.trim(), settings(), secret.trim() || undefined);
        if (!row) {
          busy = false;
          return;
        }
        added = true;
        secret = "";
      }
      step = 2;
      result = await integrations.testConnection(name.trim());
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }
</script>

<Modal bind:open title={spec ? t("integrations.connectModal.connect", { title: spec.title }) : t("integrations.connectModal.connect2")} icon="plug" size="lg">
  {#if spec && kind}
    <Stepper {steps} current={step} class="mb-5" />
    {#if step === 0}
      <div class="space-y-4">
        <div class="flex items-center gap-3">
          <BrandLogo name={spec.logo} size={44} />
          <p class="text-base text-muted">{spec.summary}</p>
        </div>
        <Field label={t("common.name")} hint={t("integrations.connectModal.usedInGrantsAnd")} error={taken ? "A connection with this name exists." : undefined}>
          {#snippet children(f)}<Input id={f.id} bind:value={name} mono invalid={taken} maxlength={32} />{/snippet}
        </Field>
        {#if signInInfo}
          <Segmented
            label={t("integrations.connectModal.how")}
            bind:value={method}
            options={[
              { value: "signin", label: t("integrations.connectModal.signInWith", { title: spec.title }), icon: "key" },
              { value: "token", label: t("integrations.connectModal.pasteAToken"), icon: "lock" },
            ]}
          />
        {/if}
        {#each fields as field (field.key)}
          <Field label={field.label} hint={field.hint} required={field.required}>
            {#snippet children(f)}
              <div class="flex gap-2">
                <Input id={f.id} class="min-w-0 flex-1" mono bind:value={values[field.key]} placeholder={field.placeholder} inputmode={field.number ? "numeric" : undefined} />
                {#if field.file}<Button icon="folder" onclick={() => choose(field.key)}>{t("common.choose")}</Button>{/if}
              </div>
            {/snippet}
          </Field>
        {/each}
        <p class="text-xs text-muted">{t("integrations.connectModal.aRunCanUse")} <span class="mono">{spec.grant}</span> {t("integrations.connectModal.onTheCommandLine")}</p>
      </div>
    {:else if step === 1 && signing && signInInfo}
      <div class="space-y-4">
        <p class="text-base text-muted">{t("integrations.connectModal.zorixAsksFor", { scopes: signInInfo.scopes })}</p>
        {#if !waiting}
          {#if signInInfo.redirect}
            <Field label={t("integrations.connectModal.redirectAddress")} hint={t("integrations.connectModal.addThisToYourApp")}>
              {#snippet children(f)}<Input id={f.id} mono readonly value={signInInfo?.redirect ?? ""} />{/snippet}
            </Field>
          {/if}
          <Field label={t("integrations.connectModal.clientId")} hint={signInInfo.builtin ? t("integrations.connectModal.leaveEmptyToUseZorixs") : undefined} required={!signInInfo.builtin}>
            {#snippet children(f)}<Input id={f.id} mono bind:value={clientId} autocomplete="off" />{/snippet}
          </Field>
          {#if signInInfo.needs_secret}
            <Field label={t("integrations.connectModal.clientSecret")} hint={t("integrations.connectModal.keptInYourKeychain")} required>
              {#snippet children(f)}<Input id={f.id} type="password" mono autocomplete="off" bind:value={clientSecret} />{/snippet}
            </Field>
          {/if}
        {:else if device}
          <Callout tone="info" icon="key" title={t("integrations.connectModal.enterThisCode", { title: spec.title })}>{device.url}</Callout>
          <p class="mono selectable text-center text-3xl tracking-widest" aria-live="polite">{device.code}</p>
          <div class="flex justify-center gap-2">
            <Button icon="copy" onclick={() => void navigator.clipboard.writeText(device?.code ?? "").catch(() => undefined)}>{t("integrations.connectModal.copyCode")}</Button>
            <Button variant="primary" icon="external" onclick={() => void connectionSignInOpen(name.trim()).catch((e) => (error = errorText(e)))}>{t("integrations.connectModal.openThePage")}</Button>
          </div>
          <p class="text-center text-sm text-muted">{t("integrations.connectModal.waitingForYou")}</p>
        {:else}
          <Callout tone="info" icon="external">{t("integrations.connectModal.finishInYourBrowser")}</Callout>
        {/if}
        {#if error}<Callout tone="err">{error}</Callout>{/if}
      </div>
    {:else if step === 1}
      <div class="space-y-4">
        {#if spec.secret}
          <Field label={spec.secret.label} hint={spec.secret.hint} required={spec.secret.required}>
            {#snippet children(f)}<Input id={f.id} type="password" mono autocomplete="off" bind:value={secret} />{/snippet}
          </Field>
          <Callout tone="muted" icon="lock">{t("integrations.connectModal.itGoesStraightTo")}</Callout>
        {:else}
          <Callout tone="ok" icon="check-circle">{t("integrations.connectModal.thisOneNeedsNo")}</Callout>
        {/if}
        {#if kind === "postgres"}<Callout tone="info" icon="shield">{t("integrations.connectModal.connectionsAreEncryptedAnd")}</Callout>{/if}
        {#if error}<Callout tone="err">{error}</Callout>{/if}
      </div>
    {:else}
      <div class="flex flex-col items-center gap-3 py-5 text-center" aria-live="polite">
        {#if !result}
          <div class="skeleton h-10 w-64"></div>
        {:else}
          <span class={result.ok ? "grid size-12 place-items-center rounded-full bg-ok/15 text-ok" : "grid size-12 place-items-center rounded-full bg-err/15 text-err"}>
            <Icon name={result.ok ? "check" : "x"} size={24} stroke={2.2} />
          </span>
          <p class="text-xl font-semibold">{result.ok ? t("integrations.connectModal.itWorks") : t("integrations.connectModal.itDidNotWork")}</p>
          <p class="selectable max-w-md text-base text-muted">{result.note}</p>
          {#if !result.ok}<p class="text-sm text-muted">{t("integrations.connectModal.theConnectionWasKept")}</p>{/if}
        {/if}
      </div>
    {/if}
  {/if}

  {#snippet footer()}
    {#if step === 0}
      <Button onclick={() => (open = false)}>{t("common.cancel")}</Button>
      <Button variant="primary" iconRight="arrow-right" disabled={!ready} onclick={() => (step = 1)}>{t("common.continue")}</Button>
    {:else if step === 1 && signing}
      {#if waiting}
        <Button onclick={() => void connectionSignInCancel(name.trim())}>{t("common.cancel")}</Button>
      {:else}
        <Button onclick={() => (step = 0)}>{t("integrations.connectModal.back")}</Button>
      {/if}
      <Button variant="primary" icon="key" loading={busy} disabled={!appReady} onclick={signIn}>{t("integrations.connectModal.signIn")}</Button>
    {:else if step === 1}
      <Button onclick={() => (step = 0)}>{t("integrations.connectModal.back")}</Button>
      <Button variant="primary" icon="check" loading={busy} disabled={!!spec?.secret?.required && !secret.trim()} onclick={addAndTest}>{t("integrations.connectModal.addAndTest")}</Button>
    {:else}
      <Button variant="primary" onclick={() => (open = false)}>{t("common.done")}</Button>
    {/if}
  {/snippet}
</Modal>
