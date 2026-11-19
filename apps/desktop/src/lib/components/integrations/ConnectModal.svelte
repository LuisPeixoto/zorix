<script lang="ts">
  import { open as pickFile } from "@tauri-apps/plugin-dialog";

  import { type ConnectionKind, errorText } from "../../api.ts";
  import { integrations } from "../../stores/integrations.svelte.ts";
  import { KINDS } from "./kinds.ts";
  import { BrandLogo, Button, Callout, Field, Icon, Input, Modal, Stepper } from "../../ui/index.ts";

  let { open = $bindable(false), kind }: { open?: boolean; kind: ConnectionKind | null } = $props();

  const steps = [{ label: "Details" }, { label: "Secret", hint: "Kept in your keychain" }, { label: "Test" }];
  let step = $state(0);
  let name = $state("");
  let values = $state<Record<string, string>>({});
  let secret = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);
  let result = $state<{ ok: boolean; note: string } | null>(null);
  let added = $state(false);

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
    }
  });

  const ready = $derived(!!spec && !!name.trim() && !taken && spec.fields.every((f) => !f.required || values[f.key]?.trim()));

  async function choose(key: string) {
    try {
      const file = await pickFile({ directory: false, multiple: false, title: "Choose the database file" });
      if (typeof file === "string") values[key] = file;
    } catch (e) {
      error = errorText(e);
    }
  }

  function settings(): Record<string, unknown> {
    const out: Record<string, unknown> = {};
    for (const field of spec?.fields ?? []) {
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

<Modal bind:open title={spec ? `Connect ${spec.title}` : "Connect"} icon="plug" size="lg">
  {#if spec && kind}
    <Stepper {steps} current={step} class="mb-5" />
    {#if step === 0}
      <div class="space-y-4">
        <div class="flex items-center gap-3">
          <BrandLogo name={spec.logo} size={44} />
          <p class="text-base text-muted">{spec.summary}</p>
        </div>
        <Field label="Name" hint="Used in grants and by the model to pick this connection." error={taken ? "A connection with this name exists." : undefined}>
          {#snippet children(f)}<Input id={f.id} bind:value={name} mono invalid={taken} maxlength={32} />{/snippet}
        </Field>
        {#each spec.fields as field (field.key)}
          <Field label={field.label} hint={field.hint} required={field.required}>
            {#snippet children(f)}
              <div class="flex gap-2">
                <Input id={f.id} class="min-w-0 flex-1" mono bind:value={values[field.key]} placeholder={field.placeholder} inputmode={field.number ? "numeric" : undefined} />
                {#if field.file}<Button icon="folder" onclick={() => choose(field.key)}>Choose…</Button>{/if}
              </div>
            {/snippet}
          </Field>
        {/each}
        <p class="text-xs text-muted">A run can use it only when you grant it, for example <span class="mono">{spec.grant}</span> on the command line, or in the New run form.</p>
      </div>
    {:else if step === 1}
      <div class="space-y-4">
        {#if spec.secret}
          <Field label={spec.secret.label} hint={spec.secret.hint} required={spec.secret.required}>
            {#snippet children(f)}<Input id={f.id} type="password" mono autocomplete="off" bind:value={secret} />{/snippet}
          </Field>
          <Callout tone="muted" icon="lock">It goes straight to your system keychain. ZORIX never shows it again, never writes it to a file and never puts it in a prompt or a log.</Callout>
        {:else}
          <Callout tone="ok" icon="check-circle">This one needs no secret.</Callout>
        {/if}
        {#if kind === "postgres"}<Callout tone="info" icon="shield">Connections are encrypted and the server's certificate is checked. Queries run in read-only transactions.</Callout>{/if}
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
          <p class="text-xl font-semibold">{result.ok ? "It works" : "It did not work"}</p>
          <p class="selectable max-w-md text-base text-muted">{result.note}</p>
          {#if !result.ok}<p class="text-sm text-muted">The connection was kept. Remove it from the list and add it again with the right details.</p>{/if}
        {/if}
      </div>
    {/if}
  {/if}

  {#snippet footer()}
    {#if step === 0}
      <Button onclick={() => (open = false)}>Cancel</Button>
      <Button variant="primary" iconRight="arrow-right" disabled={!ready} onclick={() => (step = 1)}>Continue</Button>
    {:else if step === 1}
      <Button onclick={() => (step = 0)}>Back</Button>
      <Button variant="primary" icon="check" loading={busy} disabled={!!spec?.secret?.required && !secret.trim()} onclick={addAndTest}>Add and test</Button>
    {:else}
      <Button variant="primary" onclick={() => (open = false)}>Done</Button>
    {/if}
  {/snippet}
</Modal>
