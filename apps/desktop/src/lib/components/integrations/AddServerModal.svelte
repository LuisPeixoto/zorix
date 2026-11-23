<script lang="ts">
  import { t } from "../../i18n/index.ts";
  import { open as pickFolder } from "@tauri-apps/plugin-dialog";

  import { type AddServer, type CatalogEntry, errorText } from "../../api.ts";
  import { integrations } from "../../stores/integrations.svelte.ts";
  import { BrandLogo, Button, Callout, Field, Icon, Input, KeyValue, Modal, Segmented, Stepper } from "../../ui/index.ts";

  /** Add a server from the catalogue (`entry`), or one of your own (`entry` is null). */
  let { open = $bindable(false), entry }: { open?: boolean; entry: CatalogEntry | null } = $props();

  const steps = [{ label: t("integrations.addServerModal.whatItIs") }, { label: t("integrations.addServerModal.setUp") }, { label: t("common.done") }];
  let step = $state(0);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let addedName = $state("");

  // The fields of the form. Secrets live here only until they are sent to the keychain.
  let name = $state("");
  let inputs = $state<Record<string, string>>({});
  let secrets = $state<Record<string, string>>({});
  let token = $state("");
  let custom = $state<"stdio" | "http">("stdio");
  let command = $state("");
  let argsText = $state("");
  let envText = $state("");
  let cwd = $state("");
  let url = $state("");
  let auth = $state<"none" | "bearer" | "oauth">("none");
  let clientId = $state("");

  $effect(() => {
    if (open) {
      step = 0;
      busy = false;
      error = null;
      name = entry?.default_name ?? "";
      inputs = {};
      secrets = {};
      token = "";
      custom = "stdio";
      command = argsText = envText = cwd = url = clientId = "";
      auth = "none";
    }
  });

  const isStdio = $derived(entry ? entry.transport.kind === "stdio" : custom === "stdio");
  const entryAuth = $derived(entry && entry.transport.kind === "http" ? entry.transport.auth : null);
  const needsToken = $derived(entry ? entryAuth === "bearer" : custom === "http" && auth === "bearer");
  const taken = $derived(integrations.servers.some((s) => s.name === name));

  async function choose(key: string) {
    try {
      const folder = await pickFolder({ directory: true, multiple: false, title: t("integrations.addServerModal.chooseAFolder") });
      if (typeof folder === "string") inputs[key] = folder;
    } catch (e) {
      error = errorText(e);
    }
  }

  const lines = (text: string) => text.split("\n").map((l) => l.trim()).filter(Boolean);

  async function add() {
    error = null;
    busy = true;
    try {
      const request: AddServer = { name: name.trim() || undefined };
      if (entry) {
        request.catalog = entry.id;
        request.inputs = inputs;
        request.secrets = secrets;
        if (needsToken) request.token = token;
      } else if (custom === "stdio") {
        request.command = command;
        request.args = argsText.trim() ? argsText.trim().split(/\s+/) : [];
        request.cwd = cwd || undefined;
        // One per line: NAME (a secret, asked for in the field) is not supported here, so NAME=VALUE is plain, NAME=*secret* is kept.
        request.env = lines(envText).map((line) => {
          const [key, ...rest] = line.split("=");
          const value = rest.join("=");
          return { name: key.trim(), value, secret: /(KEY|TOKEN|SECRET|PASSWORD|AUTH)/i.test(key) };
        });
      } else {
        request.url = url;
        request.auth = auth;
        if (auth === "bearer") request.token = token;
        if (auth === "oauth" && clientId.trim()) request.client_id = clientId.trim();
      }
      const row = await integrations.add(request);
      if (row) {
        addedName = row.name;
        step = 2;
      }
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function turnOn() {
    await integrations.setEnabled(addedName, true);
    open = false;
    void integrations.open(addedName);
  }
</script>

<Modal bind:open title={entry ? t("common.add", { name: entry.name }) : t("integrations.addServerModal.addYourOwnServer")} icon="plug" size="lg">
  <Stepper {steps} current={step} class="mb-5" />

  {#if step === 0}
    <div class="space-y-4">
      {#if entry}
        <div class="flex items-start gap-3">
          <BrandLogo name={entry.logo} size={44} />
          <div class="min-w-0 flex-1">
            <p class="text-lg font-semibold">{entry.name}</p>
            <p class="text-base text-muted">{entry.summary}</p>
            <a class="mt-1 inline-flex items-center gap-1 text-sm text-accent hover:underline" href={entry.homepage} target="_blank" rel="noreferrer">{entry.homepage}<Icon name="external" size={12} /></a>
          </div>
        </div>
        <KeyValue
          items={[
            { key: "How", value: entry.transport.kind === "stdio" ? "A program ZORIX starts on this computer" : "A remote server" },
            { key: "Runs", value: entry.transport.kind === "stdio" ? `${entry.transport.command} ${entry.transport.args.join(" ")}` : entry.transport.url, mono: true },
            { key: "Sign-in", value: entry.transport.kind === "http" ? { none: "None", bearer: "A token you paste", oauth: "In your browser" }[entry.transport.auth] : null },
            { key: "Needs", value: entry.requires },
          ]}
        />
        {#if entry.notes}<Callout tone="info">{entry.notes}</Callout>{/if}
      {:else}
        <Segmented
          label={t("integrations.addServerModal.kindOfServer")}
          bind:value={custom}
          options={[{ value: "stdio", label: "A program on this computer" }, { value: "http", label: "A remote server" }]}
        />
      {/if}
      {#if isStdio}
        <Callout tone="warn" icon="shield" title={t("integrations.addServerModal.thisStartsAProgram")}>
          {t("integrations.addServerModal.itRunsAsYou")}
        </Callout>
      {:else}
        <Callout tone="info" icon="globe" title={t("integrations.addServerModal.someoneElseRunsThis")}>
          {t("integrations.addServerModal.whatARunSends")}
        </Callout>
      {/if}
    </div>
  {:else if step === 1}
    <div class="space-y-4">
      <Field label={t("common.name")} hint={t("integrations.addServerModal.lowercaseLettersDigitsAnd", { value: name || 'name' })} error={taken ? "A server with this name exists." : undefined}>
        {#snippet children(f)}<Input id={f.id} bind:value={name} invalid={taken} mono maxlength={32} />{/snippet}
      </Field>
      {#if entry && entry.transport.kind === "stdio"}
        {#each entry.transport.inputs as input (input.name)}
          <Field label={input.label}>
            {#snippet children(f)}
              <div class="flex gap-2">
                <Input id={f.id} class="min-w-0 flex-1" mono bind:value={inputs[input.name]} placeholder={input.kind === "folder" ? t("common.noFolderChosen") : ""} />
                {#if input.kind === "folder"}<Button icon="folder" onclick={() => choose(input.name)}>{t("common.choose")}</Button>{/if}
              </div>
            {/snippet}
          </Field>
        {/each}
        {#each entry.transport.env.filter((v) => v.secret) as variable (variable.name)}
          <Field label={variable.label} hint={t("integrations.addServerModal.keptInYourSystem")} required>
            {#snippet children(f)}<Input id={f.id} type="password" mono autocomplete="off" bind:value={secrets[variable.name]} placeholder={variable.name} />{/snippet}
          </Field>
        {/each}
      {/if}
      {#if !entry && custom === "stdio"}
        <Field label={t("integrations.addServerModal.program")} required>
          {#snippet children(f)}<Input id={f.id} mono bind:value={command} placeholder="npx" />{/snippet}
        </Field>
        <Field label={t("common.arguments")} hint={t("integrations.addServerModal.separatedBySpaces")}>
          {#snippet children(f)}<Input id={f.id} mono bind:value={argsText} placeholder={t("integrations.addServerModal.yModelcontextprotocolServerEverything")} />{/snippet}
        </Field>
        <Field label={t("integrations.addServerModal.environment")} hint={t("integrations.addServerModal.oneNameValuePer")}>
          {#snippet children(f)}<textarea id={f.id} bind:value={envText} rows="3" class="mono w-full rounded-lg border border-line bg-panel-2 p-2.5 text-sm outline-none focus:border-accent focus:bg-panel focus:ring-3 focus:ring-accent-soft"></textarea>{/snippet}
        </Field>
      {/if}
      {#if !entry && custom === "http"}
        <Field label={t("common.address")} hint={t("integrations.addServerModal.theServerSMcp")} required>
          {#snippet children(f)}<Input id={f.id} mono bind:value={url} placeholder="https://mcp.example.com/mcp" />{/snippet}
        </Field>
        <Field label={t("integrations.addServerModal.signIn")}>
          {#snippet children(f)}
            <select id={f.id} bind:value={auth} class="h-8 w-full rounded-lg border border-line bg-panel-2 px-2.5 text-base outline-none focus:border-accent">
              <option value="none">{t("integrations.addServerModal.none")}</option>
              <option value="oauth">{t("integrations.addServerModal.inMyBrowserOauth")}</option>
              <option value="bearer">{t("integrations.addServerModal.aTokenIPaste")}</option>
            </select>
          {/snippet}
        </Field>
        {#if auth === "oauth"}
          <Field label={t("integrations.addServerModal.clientId")} hint={t("integrations.addServerModal.onlyForServersThat")}>
            {#snippet children(f)}<Input id={f.id} mono bind:value={clientId} />{/snippet}
          </Field>
        {/if}
      {/if}
      {#if needsToken}
        <Field label={t("integrations.addServerModal.token")} hint={entry?.token_help ?? t("integrations.addServerModal.keptInYourSystem")} required>
          {#snippet children(f)}<Input id={f.id} type="password" mono autocomplete="off" bind:value={token} />{/snippet}
        </Field>
      {/if}
      {#if error}<Callout tone="err">{error}</Callout>{/if}
    </div>
  {:else}
    <div class="flex flex-col items-center gap-3 py-6 text-center">
      <span class="grid size-12 place-items-center rounded-full bg-ok/15 text-ok"><Icon name="check" size={24} stroke={2.2} /></span>
      <p class="text-xl font-semibold">{t("integrations.addServerModal.wasAdded", { addedName })}</p>
      <p class="max-w-sm text-base text-muted">{t("integrations.addServerModal.itIsOffFor")}</p>
    </div>
  {/if}

  {#snippet footer()}
    {#if step === 0}
      <Button onclick={() => (open = false)}>{t("common.cancel")}</Button>
      <Button variant="primary" iconRight="arrow-right" onclick={() => (step = 1)}>{t("common.continue")}</Button>
    {:else if step === 1}
      <Button onclick={() => (step = 0)}>{t("integrations.addServerModal.back")}</Button>
      <Button variant="primary" icon="plus" loading={busy} disabled={!name.trim() || taken || (!entry && ((custom === "stdio" && !command.trim()) || (custom === "http" && !url.trim())))} onclick={add}>{t("integrations.addServerModal.addServer")}</Button>
    {:else}
      <Button onclick={() => (open = false)}>{t("integrations.addServerModal.later")}</Button>
      <Button variant="primary" icon="play" onclick={turnOn}>{t("integrations.addServerModal.turnItOnAnd")}</Button>
    {/if}
  {/snippet}
</Modal>
