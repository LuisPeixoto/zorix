<script lang="ts">
  import { type Mode, MODE_LABEL } from "../theme.ts";
  import { store } from "../stores/runs.svelte.ts";
  import { theme } from "../stores/theme.svelte.ts";
  import { type HostInfo, errorText, workflowHost, workflowSettingsSet } from "../api.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import { BrandMark, Button, Callout, Card, Field, Input, KeyValue, PageHeader, Segmented, Tabs, ToggleRow } from "../ui/index.ts";

  let tab = $state("general");
  let mode = $state<Mode>(theme.mode);
  let host = $state<HostInfo | null>(null);
  let webhooks = $state(false);
  let address = $state("");
  let saving = $state(false);
  $effect(() => {
    if (tab !== "general") return;
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
      toast.ok(host.webhooks ? `Webhooks arrive on ${host.listening ?? host.address}.` : "Webhooks are off.");
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
  <PageHeader title="Settings" subtitle="How ZORIX looks and where it keeps things. What an agent may do is set by each run and each agent." glow class="mb-1" />

  <Tabs
    tabs={[
      { value: "general", label: "General" },
      { value: "appearance", label: "Appearance" },
      { value: "about", label: "About" },
    ]}
    bind:value={tab}
    label="Settings"
  />

  {#if tab === "general"}
    <Card title="Data" icon="database">
      <KeyValue
        items={[
          { key: "Data folder", value: store.info?.data_dir, mono: true },
          { key: "Version", value: store.info?.version, mono: true },
        ]}
      />
      <p class="mt-3 text-sm text-muted">Runs, memory, agents and settings live in this folder, on this computer. Keys are kept in the system keychain, never in it.</p>
    </Card>
    <Card title="Webhooks" icon="webhook">
      <p class="mb-3 text-base text-muted">A workflow can start when a web request arrives. Nothing listens until you turn this on. Schedules and events need no setting: they run while ZORIX is open, or under <span class="mono">zorix serve</span>.</p>
      <ToggleRow label="Receive webhooks" description={webhooks ? "ZORIX listens for requests to a workflow's secret address" : "Nothing listens"} bind:checked={webhooks} />
      {#if webhooks}
        <div class="mt-3 max-w-sm">
          <Field label="Address" hint="127.0.0.1 is this computer only. To receive from the internet, put a tunnel or a reverse proxy in front.">
            {#snippet children(f)}<Input id={f.id} mono bind:value={address} placeholder="127.0.0.1:7788" />{/snippet}
          </Field>
        </div>
      {/if}
      {#if host && !host.running}<div class="mt-3"><Callout tone="warn" title="The workflow host is not running">Schedules and webhooks are off until ZORIX is opened again.</Callout></div>{/if}
      <div class="mt-4 flex items-center gap-3">
        <Button variant="primary" disabled={!changed || saving} onclick={() => void saveWebhooks()}>Save</Button>
        {#if host?.listening}<span class="mono text-xs text-muted">Listening on {host.listening}</span>{/if}
      </div>
    </Card>
  {:else if tab === "appearance"}
    <Card title="Theme" icon="sun">
      <p class="mb-3 text-base text-muted">Light is the default. System follows your computer's setting and changes with it.</p>
      <Segmented
        label="Theme"
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
        <p class="max-w-[46ch] text-base text-muted">A harness for AI: it decides what a model may see and do, stops to ask a person before anything changes, and records every step.</p>
        <p class="mono text-sm text-muted">Version {store.info?.version ?? ""}</p>
        <p class="text-xs text-muted">MIT licence. Fonts and brand marks keep their own licences; see THIRD-PARTY.md in the repository.</p>
      </div>
    </Card>
  {/if}
</div>
