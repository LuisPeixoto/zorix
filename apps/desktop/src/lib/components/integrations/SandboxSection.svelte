<script lang="ts">
  import { open as pickFolder } from "@tauri-apps/plugin-dialog";
  import { t as tr } from "../../i18n/index.ts";
  import type { SandboxFolder, ServerDetail } from "../../api.ts";
  import { integrations } from "../../stores/integrations.svelte.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { Button, Callout, IconButton, KeyValue, Segmented, Switch, ToggleRow } from "../../ui/index.ts";

  /** How a local server is confined: the mode, the network, the folders, and what this computer really enforces. */
  let { detail }: { detail: ServerDetail } = $props();

  let mode = $state<"native" | "off">("off");
  let network = $state(false);
  let folders = $state<SandboxFolder[]>([]);
  let saving = $state(false);

  // The form follows the server on screen: its name and its stored choice, not the object, which is replaced on every poll.
  const stored = $derived(JSON.stringify(detail.sandbox));
  $effect(() => {
    void detail.name;
    const sandbox = JSON.parse(stored) as ServerDetail["sandbox"];
    mode = sandbox?.mode ?? "off";
    network = sandbox?.network ?? false;
    folders = sandbox?.folders ?? [];
  });

  const changed = $derived(detail.sandbox === null || JSON.stringify({ mode, network, folders }) !== stored);
  const word = (enforced: boolean) => (enforced ? tr("integrations.sandbox.enforced") : tr("integrations.sandbox.notEnforced"));
  const rows = $derived([
    { key: tr("integrations.sandbox.files"), value: `${word(detail.enforced.files)}: ${tr("integrations.sandbox.filesText")}` },
    { key: tr("integrations.sandbox.network"), value: `${word(detail.enforced.network)}: ${tr("integrations.sandbox.networkText")}` },
    { key: tr("integrations.sandbox.credentials"), value: `${word(detail.enforced.credentials)}: ${tr("integrations.sandbox.credentialsText")}` },
    { key: tr("integrations.sandbox.processes"), value: `${word(detail.enforced.processes)}: ${tr("integrations.sandbox.processesText")}` },
  ]);

  async function add() {
    const folder = await pickFolder({ directory: true, multiple: false, title: tr("integrations.sandbox.chooseAFolder") });
    if (typeof folder === "string" && !folders.some((f) => f.path === folder)) folders = [...folders, { path: folder, write: false }];
  }

  async function save() {
    saving = true;
    if (await integrations.setSandbox(detail.name, { mode, network, folders })) toast.ok(tr("integrations.sandbox.saved", { name: detail.name }));
    saving = false;
  }
</script>

<section class="space-y-3" aria-label={tr("integrations.sandbox.title")}>
  <p class="text-sm font-medium">{tr("integrations.sandbox.title")}</p>
  {#if detail.sandbox === null}
    <Callout tone="warn" icon="shield" title={tr("integrations.sandbox.notChosen")}>{tr("integrations.sandbox.notChosenText")}</Callout>
  {/if}
  <Segmented
    bind:value={mode}
    label={tr("integrations.sandbox.how")}
    options={[
      { value: "native", label: tr("integrations.sandbox.native") },
      { value: "off", label: tr("integrations.sandbox.off") },
    ]}
  />
  {#if mode === "off"}
    <Callout tone="muted" icon="info">{tr("integrations.sandbox.offText")}</Callout>
  {:else}
    {#if !detail.enforced.files}
      <Callout tone="warn" icon="alert">{tr("integrations.sandbox.windows")}</Callout>
    {/if}
    <ToggleRow class="px-0" label={tr("integrations.sandbox.network")} description={tr("integrations.sandbox.networkHint")} bind:checked={network} />
    <div>
      <p class="mb-1.5 text-sm font-medium">{tr("integrations.sandbox.foldersItMayUse")}</p>
      {#if folders.length === 0}
        <p class="text-sm text-muted">{tr("integrations.sandbox.noFolders")}</p>
      {:else}
        <ul class="space-y-1">
          {#each folders as folder (folder.path)}
            <li class="flex items-center gap-2 text-sm">
              <span class="mono min-w-0 flex-1 truncate" title={folder.path}>{folder.path}</span>
              <span class="text-muted">{tr("integrations.sandbox.mayWrite")}</span>
              <Switch bind:checked={folder.write} aria-label={tr("integrations.sandbox.mayWrite")} />
              <IconButton icon="x" label={tr("integrations.sandbox.remove", { path: folder.path })} onclick={() => (folders = folders.filter((f) => f.path !== folder.path))} />
            </li>
          {/each}
        </ul>
      {/if}
      <Button class="mt-2" size="sm" icon="folder" onclick={add}>{tr("integrations.sandbox.addAFolder")}</Button>
    </div>
    <div>
      <p class="mb-1.5 text-sm font-medium">{tr("integrations.sandbox.enforcedOn")}</p>
      <KeyValue items={rows} />
    </div>
  {/if}
  <div class="flex justify-end">
    <Button variant="primary" disabled={!changed || saving} onclick={save}>{tr("integrations.sandbox.save")}</Button>
  </div>
</section>
