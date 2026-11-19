<script lang="ts">
  import { open as pickFolder } from "@tauri-apps/plugin-dialog";

  import { type AddServer, type CatalogEntry, errorText } from "../../api.ts";
  import { integrations } from "../../stores/integrations.svelte.ts";
  import { BrandLogo, Button, Callout, Field, Icon, Input, KeyValue, Modal, Segmented, Stepper } from "../../ui/index.ts";

  /** Add a server from the catalogue (`entry`), or one of your own (`entry` is null). */
  let { open = $bindable(false), entry }: { open?: boolean; entry: CatalogEntry | null } = $props();

  const steps = [{ label: "What it is" }, { label: "Set up" }, { label: "Done" }];
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
      const folder = await pickFolder({ directory: true, multiple: false, title: "Choose a folder" });
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

<Modal bind:open title={entry ? `Add ${entry.name}` : "Add your own server"} icon="plug" size="lg">
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
          label="Kind of server"
          bind:value={custom}
          options={[{ value: "stdio", label: "A program on this computer" }, { value: "http", label: "A remote server" }]}
        />
      {/if}
      {#if isStdio}
        <Callout tone="warn" icon="shield" title="This starts a program on your computer">
          It runs as you, with your permissions and no sandbox. ZORIX decides which of its tools a run may use and asks before each call, but cannot limit what the program itself does. Only add one you trust.
        </Callout>
      {:else}
        <Callout tone="info" icon="globe" title="Someone else runs this server">
          What a run sends to its tools reaches that server, and its tool descriptions are written by its operator. ZORIX shows you each description and tells you when one changes.
        </Callout>
      {/if}
    </div>
  {:else if step === 1}
    <div class="space-y-4">
      <Field label="Name" hint="Lowercase letters, digits, - and _. It appears in tool ids such as mcp.{name || 'name'}.search" error={taken ? "A server with this name exists." : undefined}>
        {#snippet children(f)}<Input id={f.id} bind:value={name} invalid={taken} mono maxlength={32} />{/snippet}
      </Field>
      {#if entry && entry.transport.kind === "stdio"}
        {#each entry.transport.inputs as input (input.name)}
          <Field label={input.label}>
            {#snippet children(f)}
              <div class="flex gap-2">
                <Input id={f.id} class="min-w-0 flex-1" mono bind:value={inputs[input.name]} placeholder={input.kind === "folder" ? "No folder chosen" : ""} />
                {#if input.kind === "folder"}<Button icon="folder" onclick={() => choose(input.name)}>Choose…</Button>{/if}
              </div>
            {/snippet}
          </Field>
        {/each}
        {#each entry.transport.env.filter((v) => v.secret) as variable (variable.name)}
          <Field label={variable.label} hint="Kept in your system keychain, never in a file." required>
            {#snippet children(f)}<Input id={f.id} type="password" mono autocomplete="off" bind:value={secrets[variable.name]} placeholder={variable.name} />{/snippet}
          </Field>
        {/each}
      {/if}
      {#if !entry && custom === "stdio"}
        <Field label="Program" required>
          {#snippet children(f)}<Input id={f.id} mono bind:value={command} placeholder="npx" />{/snippet}
        </Field>
        <Field label="Arguments" hint="Separated by spaces.">
          {#snippet children(f)}<Input id={f.id} mono bind:value={argsText} placeholder="-y @modelcontextprotocol/server-everything" />{/snippet}
        </Field>
        <Field label="Environment" hint="One NAME=VALUE per line. Names that look like a key, token or password go to the keychain.">
          {#snippet children(f)}<textarea id={f.id} bind:value={envText} rows="3" class="mono w-full rounded-lg border border-line bg-panel-2 p-2.5 text-sm outline-none focus:border-accent focus:bg-panel focus:ring-3 focus:ring-accent-soft"></textarea>{/snippet}
        </Field>
      {/if}
      {#if !entry && custom === "http"}
        <Field label="Address" hint="The server's MCP endpoint, over HTTPS." required>
          {#snippet children(f)}<Input id={f.id} mono bind:value={url} placeholder="https://mcp.example.com/mcp" />{/snippet}
        </Field>
        <Field label="Sign-in">
          {#snippet children(f)}
            <select id={f.id} bind:value={auth} class="h-8 w-full rounded-lg border border-line bg-panel-2 px-2.5 text-base outline-none focus:border-accent">
              <option value="none">None</option>
              <option value="oauth">In my browser (OAuth)</option>
              <option value="bearer">A token I paste</option>
            </select>
          {/snippet}
        </Field>
        {#if auth === "oauth"}
          <Field label="Client id" hint="Only for servers that need one registered in advance. Most do not.">
            {#snippet children(f)}<Input id={f.id} mono bind:value={clientId} />{/snippet}
          </Field>
        {/if}
      {/if}
      {#if needsToken}
        <Field label="Token" hint={entry?.token_help ?? "Kept in your system keychain, never in a file."} required>
          {#snippet children(f)}<Input id={f.id} type="password" mono autocomplete="off" bind:value={token} />{/snippet}
        </Field>
      {/if}
      {#if error}<Callout tone="err">{error}</Callout>{/if}
    </div>
  {:else}
    <div class="flex flex-col items-center gap-3 py-6 text-center">
      <span class="grid size-12 place-items-center rounded-full bg-ok/15 text-ok"><Icon name="check" size={24} stroke={2.2} /></span>
      <p class="text-xl font-semibold">{addedName} was added</p>
      <p class="max-w-sm text-base text-muted">It is off for now. When you turn it on, ZORIX starts it, reads its tools and waits for you to approve them before any run can use one.</p>
    </div>
  {/if}

  {#snippet footer()}
    {#if step === 0}
      <Button onclick={() => (open = false)}>Cancel</Button>
      <Button variant="primary" iconRight="arrow-right" onclick={() => (step = 1)}>Continue</Button>
    {:else if step === 1}
      <Button onclick={() => (step = 0)}>Back</Button>
      <Button variant="primary" icon="plus" loading={busy} disabled={!name.trim() || taken || (!entry && ((custom === "stdio" && !command.trim()) || (custom === "http" && !url.trim())))} onclick={add}>Add server</Button>
    {:else}
      <Button onclick={() => (open = false)}>Later</Button>
      <Button variant="primary" icon="play" onclick={turnOn}>Turn it on and review its tools</Button>
    {/if}
  {/snippet}
</Modal>
