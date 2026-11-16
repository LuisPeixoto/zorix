<script lang="ts">
  import { keyLabel } from "../format.ts";
  import { store } from "../stores/runs.svelte.ts";

  // The typed key lives only in this field until it is sent to the keychain, then it is cleared.
  let drafts = $state<Record<string, string>>({});

  async function save(name: string) {
    const key = drafts[name] ?? "";
    if (!key.trim()) return;
    if (await store.saveKey(name, key)) drafts[name] = "";
  }

  const tone = (key: string) => (key === "present" ? "text-ok" : key === "missing" ? "text-warn" : key === "unavailable" ? "text-err" : "text-faint");
</script>

<div class="mx-auto flex h-full max-w-3xl flex-col gap-5 overflow-y-auto p-8">
  <div>
    <h2 class="text-3xl font-semibold">Providers</h2>
    <p class="mt-1 text-muted">
      Keys are kept in your system keychain, never in a file or in this window. Cloud providers receive the conversation;
      a provider marked <strong>local</strong> keeps it on this computer.
    </p>
  </div>

  {#if store.error}<p class="text-err" role="alert">{store.error}</p>{/if}
  {#if store.providerNote}<p class="text-ok" role="status">{store.providerNote}</p>{/if}

  <ul class="divide-y divide-line rounded-lg border border-line bg-panel">
    {#each store.providers as p (p.name)}
      <li class="p-4">
        <div class="flex items-baseline justify-between gap-3">
          <span class="font-semibold">{p.name}</span>
          <span class="text-xs {tone(p.key)}">{keyLabel(p.key)}{p.enabled ? "" : " · off"}</span>
        </div>
        <p class="mt-0.5 truncate font-mono text-xs text-faint" title={p.url}>
          {p.kind} · {p.local ? "local" : "cloud"} · {p.url}
        </p>
        {#if p.key_note}<p class="mt-1 text-xs text-err">{p.key_note}</p>{/if}
        <div class="mt-3 flex flex-wrap items-center gap-2">
          {#if p.key !== "not_needed" || !p.local}
            <input
              class="min-w-0 flex-1 rounded-md border border-line bg-bg px-3 py-1.5 font-mono text-xs"
              type="password"
              autocomplete="off"
              spellcheck="false"
              placeholder={p.key === "present" ? "Paste a new key to replace it" : "Paste the key"}
              aria-label="Key for {p.name}"
              bind:value={drafts[p.name]}
            />
            <button class="rounded-md bg-accent px-3 py-1.5 font-medium text-accent-fg" disabled={store.busy || !(drafts[p.name] ?? "").trim()} onclick={() => save(p.name)}>Save key</button>
          {/if}
          <button class="rounded-md border border-line px-3 py-1.5 hover:bg-hover" disabled={store.busy} onclick={() => store.check(p.name)}>Test</button>
          {#if p.key === "present"}
            <button class="rounded-md border border-line px-3 py-1.5 text-err hover:bg-hover" disabled={store.busy} onclick={() => store.deleteKey(p.name)}>Remove key</button>
          {/if}
        </div>
      </li>
    {/each}
  </ul>
  <p class="text-xs text-faint">
    Routes, aliases and extra servers (OpenRouter, Groq, LM Studio…) are set in providers.toml in the data folder.
  </p>
</div>
