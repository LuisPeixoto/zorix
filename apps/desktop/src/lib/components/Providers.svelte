<script lang="ts">
  import { keyLabel, keyTone } from "../format.ts";
  import Embeddings from "./Embeddings.svelte";
  import { store } from "../stores/runs.svelte.ts";
  import { Badge, Button, Callout, Card, Chip, Icon, Input, Modal, PageHeader } from "../ui/index.ts";

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

  import { agents } from "../stores/agents.svelte.ts";
  import { providerName } from "../newRun.ts";

  let picked = $state<string | null>(null);
  const current = $derived(store.providers.find((p) => p.name === picked) ?? store.providers[0]);
  const models = $derived(current ? (store.models?.models ?? []).filter((m) => m.reference.startsWith(`${current.name}/`)) : []);
  const usedBy = (reference: string) => agents.cards.filter((c) => c.default_model === reference).length;
  const contextText = (tokens: number) => (tokens >= 1_000_000 ? `${tokens / 1_000_000}M` : `${Math.round(tokens / 1000)}k`);
</script>

<div class="h-full overflow-y-auto">
  <div class="mx-auto max-w-[1100px] p-6">
    <PageHeader title="Providers" subtitle="Where models come from. Keys live in the system keychain; routes fall back when a provider fails." glow />

    {#if store.error}<Callout tone="err" class="mb-4">{store.error}</Callout>{/if}

    <div class="grid items-start gap-5 lg:grid-cols-[300px_minmax(0,1fr)]">
      <ul class="space-y-2" aria-label="Providers">
        {#each store.providers as p (p.name)}
          {@const on = current?.name === p.name}
          <li>
            <button type="button" aria-pressed={on} onclick={() => (picked = p.name)} class="flex w-full items-center gap-3 rounded-xl border px-3 py-2.5 text-left transition duration-150 {on ? 'border-accent/50 bg-accent-soft' : 'border-line bg-panel hover:border-line-strong'}">
              <span class="grid size-8 shrink-0 place-items-center rounded-lg border border-line bg-panel-2 text-muted"><Icon name={p.local ? "laptop" : "cloud"} size={15} /></span>
              <span class="min-w-0 flex-1">
                <span class="block truncate text-md font-semibold">{providerName(p.name)}</span>
                <span class="mono block truncate text-xs text-muted" title={p.url}>{p.url.replace(/^https?:\/\//, "")}</span>
              </span>
              <Badge tone={keyTone(p.key)} dot>{keyLabel(p.key)}</Badge>
            </button>
          </li>
        {/each}
      </ul>

      {#if current}
        {@const p = current}
        <div class="min-w-0 space-y-5">
          <Card title={providerName(p.name)} icon={p.local ? "laptop" : "cloud"}>
            {#snippet actions()}
              <Badge tone={p.local ? "accent" : "muted"}>{p.local ? "local" : "cloud"}</Badge>
              {#if !p.enabled}<Badge tone="muted">off</Badge>{/if}
              <Button size="sm" icon="refresh" disabled={store.busy} onclick={() => store.check(p.name)}>Test</Button>
            {/snippet}
            <p class="selectable mono truncate text-sm text-muted" title={p.url}>{p.kind} · {p.url}</p>
            {#if p.key_note}<p class="mt-1 text-sm text-err">{p.key_note}</p>{/if}
            <div class="mt-4">
              <p class="mb-1.5 text-sm font-medium">API key</p>
              {#if p.key !== "not_needed" || !p.local}
                <div class="flex flex-wrap items-center gap-2">
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
                  {#if p.key === "present"}<Button variant="danger" icon="trash" disabled={store.busy} onclick={() => askRemove(p.name)}>Remove key</Button>{/if}
                </div>
                <p class="mt-2 flex items-center gap-1.5 text-xs text-muted"><Icon name="lock" size={12} />Stored in the system keychain, never in a file or in an event.</p>
              {:else}
                <p class="text-sm text-muted">No key needed. Start the server and press Test.</p>
              {/if}
            </div>
          </Card>

          <Card title="Models" icon="cpu" padding="none">
            {#if models.length === 0}
              <p class="px-4 py-6 text-center text-base text-muted">No models are listed for {providerName(p.name)} yet. Press Test to look them up.</p>
            {:else}
              <div class="overflow-x-auto">
                <table class="w-full text-left text-base">
                  <caption class="sr-only">Models of {providerName(p.name)}</caption>
                  <thead>
                    <tr class="border-b border-line-soft bg-panel-2 text-2xs font-semibold tracking-wider text-faint uppercase">
                      <th scope="col" class="px-4 py-2">Model</th>
                      <th scope="col" class="px-3 py-2">Context</th>
                      <th scope="col" class="px-3 py-2">Can</th>
                      <th scope="col" class="px-3 py-2">Price</th>
                      <th scope="col" class="px-4 py-2">Used by</th>
                    </tr>
                  </thead>
                  <tbody class="divide-y divide-line-soft">
                    {#each models as m (m.reference)}
                      <tr>
                        <td class="mono px-4 py-2.5 text-sm font-medium">{m.name}</td>
                        <td class="mono tnum px-3 py-2.5 text-sm text-muted">{contextText(m.context_window)}</td>
                        <td class="px-3 py-2.5 text-sm text-muted">{[m.tools && "tools", m.vision && "images"].filter(Boolean).join(", ") || "text"}</td>
                        <td class="px-3 py-2.5 text-sm text-muted">{m.local ? "free" : m.price_known ? "priced" : "unknown"}</td>
                        <td class="px-4 py-2.5 text-sm text-muted">{usedBy(m.reference) ? `${usedBy(m.reference)} agent${usedBy(m.reference) === 1 ? "" : "s"}` : "none"}</td>
                      </tr>
                    {/each}
                  </tbody>
                </table>
              </div>
            {/if}
          </Card>

          <Card title="Routes and fallback" icon="route">
            {#if (store.models?.routes.length ?? 0) + (store.models?.aliases.length ?? 0) === 0}
              <p class="text-sm text-muted">No routes or aliases yet. A route names a list of models to try in order, so a run keeps going when one provider fails.</p>
            {:else}
              <div class="flex flex-wrap gap-2">
                {#each store.models?.routes ?? [] as r (r)}<Chip><Icon name="route" size={12} /><span class="mono">route:{r}</span></Chip>{/each}
                {#each store.models?.aliases ?? [] as a (a)}<Chip><span class="mono">{a}</span></Chip>{/each}
              </div>
            {/if}
            <p class="mt-3 text-xs text-muted">Routes, aliases and extra servers (OpenRouter, Groq, LM Studio…) are set in providers.toml in the data folder.</p>
          </Card>
        </div>
      {/if}
    </div>

    <div class="mt-5"><Embeddings /></div>
  </div>
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
