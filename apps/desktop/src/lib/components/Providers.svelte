<script lang="ts">
  import { keyLabel, keyTone } from "../format.ts";
  import Embeddings from "./Embeddings.svelte";
  import { store } from "../stores/runs.svelte.ts";
  import { Badge, Button, Callout, Card, Icon, Input, Modal, PageHeader } from "../ui/index.ts";

  // The typed key lives only in this field until it is sent to the keychain, then it is cleared.
  let drafts = $state<Record<string, string>>({});
  let removing = $state<string | null>(null);
  let confirmOpen = $state(false);

  async function save(name: string) {
    const key = drafts[name] ?? "";
    if (!key.trim()) return;
    if (await store.saveKey(name, key)) drafts[name] = "";
  }

  function askRemove(name: string) {
    removing = name;
    confirmOpen = true;
  }

  async function remove() {
    const name = removing;
    confirmOpen = false;
    if (name) await store.deleteKey(name);
  }
</script>

<div class="mx-auto flex h-full max-w-3xl flex-col gap-5 overflow-y-auto p-8 [&>*]:shrink-0">
  <PageHeader title="Providers &amp; keys" subtitle="Where the models come from, and the keys that let ZORIX use them." glow class="mb-1" />

  <Callout tone="info" icon="lock" title="Keys stay on this computer">
    They are kept in your system keychain, never in a file or in this window. Cloud providers receive the conversation of a run; a provider marked <strong class="text-fg">local</strong> keeps it here.
  </Callout>

  {#if store.error}<Callout tone="err">{store.error}</Callout>{/if}

  <div class="space-y-3">
    {#each store.providers as p (p.name)}
      <Card>
        <div class="flex items-start gap-3">
          <span class="grid size-9 shrink-0 place-items-center rounded-xl bg-accent-soft text-accent">
            <Icon name={p.local ? "laptop" : "cloud"} size={17} />
          </span>
          <div class="min-w-0 flex-1">
            <div class="flex flex-wrap items-center gap-2">
              <h3 class="text-lg font-semibold">{p.name}</h3>
              <Badge tone={p.local ? "accent" : "muted"}>{p.local ? "local" : "cloud"}</Badge>
              {#if !p.enabled}<Badge tone="muted">off</Badge>{/if}
            </div>
            <p class="selectable mono mt-0.5 truncate text-xs text-faint" title={p.url}>{p.kind} · {p.url}</p>
            {#if p.key_note}<p class="mt-1 text-xs text-err">{p.key_note}</p>{/if}
          </div>
          <Badge tone={keyTone(p.key)} dot>{keyLabel(p.key)}</Badge>
        </div>

        <div class="mt-4 flex flex-wrap items-center gap-2">
          {#if p.key !== "not_needed" || !p.local}
            <Input
              class="min-w-0 flex-1"
              type="password"
              mono
              icon="key"
              autocomplete="off"
              spellcheck={false}
              placeholder={p.key === "present" ? "Paste a new key to replace it" : "Paste the key"}
              aria-label="Key for {p.name}"
              bind:value={() => drafts[p.name] ?? "", (v) => (drafts[p.name] = v)}
              onkeydown={(e) => e.key === "Enter" && save(p.name)}
            />
            <Button variant="primary" icon="check" disabled={store.busy || !(drafts[p.name] ?? "").trim()} onclick={() => save(p.name)}>Save key</Button>
          {:else}
            <p class="flex-1 text-sm text-muted">No key needed. Start the server and press Test.</p>
          {/if}
          <Button icon="refresh" disabled={store.busy} onclick={() => store.check(p.name)}>Test</Button>
          {#if p.key === "present"}
            <Button variant="danger" icon="trash" disabled={store.busy} onclick={() => askRemove(p.name)}>Remove key</Button>
          {/if}
        </div>
      </Card>
    {/each}
  </div>

  <Embeddings />

  <p class="flex items-start gap-2 text-xs text-faint">
    <Icon name="info" size={14} class="mt-px" />
    Routes, aliases and extra servers (OpenRouter, Groq, LM Studio…) are set in providers.toml in the data folder.
  </p>
</div>

<Modal bind:open={confirmOpen} title="Remove the key?" icon="key" size="sm">
  <p class="text-base text-muted">
    The key for <strong class="text-fg">{removing}</strong> is deleted from the system keychain. You can paste it again later.
  </p>
  {#snippet footer()}
    <Button onclick={() => (confirmOpen = false)}>Cancel</Button>
    <Button variant="danger" icon="trash" onclick={remove}>Remove key</Button>
  {/snippet}
</Modal>
