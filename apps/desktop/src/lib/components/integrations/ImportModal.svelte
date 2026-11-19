<script lang="ts">
  import { type ImportPreview, errorText, mcpImportApply, mcpImportPreview } from "../../api.ts";
  import { integrations } from "../../stores/integrations.svelte.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { Badge, Button, Callout, Checkbox, Modal, Segmented } from "../../ui/index.ts";

  let { open = $bindable(false) }: { open?: boolean } = $props();

  let source = $state("claude");
  let previews = $state<ImportPreview[]>([]);
  let chosen = $state<Record<string, boolean>>({});
  let busy = $state(false);
  let error = $state<string | null>(null);
  let looked = $state(false);

  async function look() {
    error = null;
    looked = false;
    try {
      previews = await mcpImportPreview(source);
      chosen = {};
      for (const preview of previews) for (const row of preview.rows) if (!row.skipped) chosen[`${preview.path}\n${row.name}`] = true;
    } catch (e) {
      previews = [];
      error = errorText(e);
    }
    looked = true;
  }

  $effect(() => {
    if (open) {
      previews = [];
      looked = false;
      void look();
    }
  });
  $effect(() => {
    void source;
    if (open) void look();
  });

  const count = $derived(Object.values(chosen).filter(Boolean).length);

  async function apply() {
    busy = true;
    try {
      const messages: string[] = [];
      for (const preview of previews) {
        const names = preview.rows.filter((r) => chosen[`${preview.path}\n${r.name}`]).map((r) => r.name);
        if (names.length) messages.push(...(await mcpImportApply(preview.source, preview.path, names)));
      }
      await integrations.refresh();
      toast.ok(messages.join(" "));
      open = false;
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }
</script>

<Modal bind:open title="Import servers" icon="download" size="lg">
  <div class="space-y-4">
    <p class="text-base text-muted">Bring in the MCP servers you already set up in another program. ZORIX reads its configuration only now, only the file you pick, and shows what it found before changing anything.</p>
    <Segmented
      label="Import from"
      bind:value={source}
      options={[{ value: "claude", label: "Claude Desktop" }, { value: "vscode", label: "VS Code" }, { value: "cursor", label: "Cursor" }]}
    />
    {#if error}<Callout tone="err">{error}</Callout>{/if}
    {#if looked && previews.length === 0 && !error}
      <Callout tone="muted" icon="info">No configuration was found in the usual place for this program.</Callout>
    {/if}
    {#each previews as preview (preview.path)}
      <div class="rounded-xl border border-line">
        <p class="mono truncate border-b border-line-soft px-3 py-2 text-xs text-muted" title={preview.path}>{preview.path}</p>
        <ul class="divide-y divide-line-soft">
          {#each preview.rows as row (row.name)}
            <li class="flex items-start gap-3 px-3 py-2.5">
              {#if row.skipped}
                <Badge tone="muted">skipped</Badge>
              {:else}
                <Checkbox checked={chosen[`${preview.path}\n${row.name}`]} onchange={(e) => (chosen[`${preview.path}\n${row.name}`] = e.currentTarget.checked)} aria-label="Import {row.name}" />
              {/if}
              <div class="min-w-0 flex-1">
                <p class="text-md font-medium">{row.name}{#if row.original_name !== row.name}<span class="ml-2 text-xs text-muted">was “{row.original_name}”</span>{/if}</p>
                <p class="mono truncate text-xs text-muted">{row.summary ?? row.skipped}</p>
                {#each row.warnings as warning}<p class="mt-0.5 text-xs text-warn">{warning}</p>{/each}
                {#if row.secrets}<p class="mt-0.5 text-xs text-muted">{row.secrets} secret{row.secrets === 1 ? "" : "s"} will move to your keychain.</p>{/if}
              </div>
            </li>
          {/each}
        </ul>
      </div>
    {/each}
    {#if count > 0}<Callout tone="info" icon="shield">Imported servers start off. You turn each one on yourself, after seeing what it runs.</Callout>{/if}
  </div>
  {#snippet footer()}
    <Button onclick={() => (open = false)}>Cancel</Button>
    <Button variant="primary" icon="download" loading={busy} disabled={count === 0} onclick={apply}>Import {count || ""}</Button>
  {/snippet}
</Modal>
