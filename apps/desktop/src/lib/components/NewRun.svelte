<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";

  import { errorText } from "../api.ts";
  import { store } from "../stores/runs.svelte.ts";

  const examples = [
    "What is 17*23 and save it as result",
    "What time is it?",
    "Recall result",
    "Count the words in \"the quick brown fox\"",
  ];

  let prompt = $state("");
  let workspace = $state("");
  let allowRead = $state("*");
  let allowWrite = $state("");
  // What the person changed from the defaults. Offer the tools that need nothing more than a tool permission; file tools
  // need a workspace, so they start off.
  let overrides = $state<Record<string, boolean>>({});
  let pickError = $state<string | null>(null);

  const isOn = (id: string) => overrides[id] ?? !id.startsWith("workspace.");

  const how = (t: { requires_approval: boolean; annotations: { read_only: boolean } }) =>
    t.requires_approval ? "asks you first" : t.annotations.read_only ? "read-only" : "changes things";

  async function pickWorkspace() {
    pickError = null;
    try {
      const folder = await open({ directory: true, multiple: false, title: "Choose the workspace folder" });
      if (typeof folder === "string") workspace = folder;
    } catch (e) {
      pickError = errorText(e);
    }
  }

  const lines = (text: string) => text.split(/[\n,]/).map((s) => s.trim()).filter(Boolean);

  function submit(event: Event) {
    event.preventDefault();
    void store.start({
      prompt: prompt.trim(),
      allow_tools: store.tools.map((t) => t.id).filter(isOn),
      allow_read: workspace ? lines(allowRead) : [],
      allow_write: workspace ? lines(allowWrite) : [],
      workspace: workspace || undefined,
    });
  }
</script>

<form class="mx-auto flex h-full max-w-3xl flex-col gap-5 overflow-y-auto p-8" onsubmit={submit}>
  <div>
    <h2 class="text-3xl font-semibold">New run</h2>
    <p class="mt-1 text-muted">
      Say what you want done. The run can only use what you allow below, and anything that changes something asks you first.
    </p>
  </div>

  <div>
    <label class="mb-1 block font-medium" for="prompt">What should it do?</label>
    <textarea
      id="prompt"
      class="h-28 w-full resize-none rounded-lg border border-line bg-panel p-3 text-base"
      placeholder="For example: What is 17*23 and save it as result"
      bind:value={prompt}
      maxlength="20000"
    ></textarea>
    <div class="mt-2 flex flex-wrap gap-2">
      {#each examples as example}
        <button type="button" class="rounded-full border border-line px-3 py-1 text-xs text-muted hover:bg-hover" onclick={() => (prompt = example)}>{example}</button>
      {/each}
    </div>
  </div>

  <fieldset>
    <legend class="mb-1 font-medium">Tools it may use</legend>
    <ul class="divide-y divide-line rounded-lg border border-line bg-panel">
      {#each store.tools as tool (tool.id)}
        <li>
          <label class="flex cursor-pointer items-start gap-3 px-3 py-2 hover:bg-hover">
            <input
              class="mt-1"
              type="checkbox"
              checked={isOn(tool.id)}
              onchange={(e) => (overrides[tool.id] = e.currentTarget.checked)}
            />
            <span class="min-w-0 flex-1">
              <span class="font-mono text-xs">{tool.id}</span>
              <span class="ml-2 text-xs {tool.requires_approval ? 'text-warn' : 'text-faint'}">{how(tool)}</span>
              <span class="block text-muted">{tool.description}</span>
            </span>
          </label>
        </li>
      {/each}
    </ul>
  </fieldset>

  <fieldset class="rounded-lg border border-line bg-panel p-4">
    <legend class="px-1 font-medium">Workspace folder (for the file tools)</legend>
    <div class="flex gap-2">
      <input class="min-w-0 flex-1 rounded-md border border-line bg-bg px-3 py-1.5 font-mono text-xs" placeholder="No folder: file tools will refuse every call" bind:value={workspace} />
      <button type="button" class="rounded-md border border-line px-3 py-1.5 hover:bg-hover" onclick={pickWorkspace}>Choose…</button>
    </div>
    {#if pickError}<p class="mt-1 text-err">{pickError}</p>{/if}
    {#if workspace}
      <div class="mt-3 grid grid-cols-2 gap-3">
        <div>
          <label class="text-xs text-muted" for="read">May read (patterns)</label>
          <input id="read" class="mt-1 w-full rounded-md border border-line bg-bg px-3 py-1.5 font-mono text-xs" bind:value={allowRead} placeholder="* or docs/*" />
        </div>
        <div>
          <label class="text-xs text-muted" for="write">May write (patterns)</label>
          <input id="write" class="mt-1 w-full rounded-md border border-line bg-bg px-3 py-1.5 font-mono text-xs" bind:value={allowWrite} placeholder="none, or out/*" />
        </div>
      </div>
    {/if}
  </fieldset>

  {#if store.error}<p class="text-err" role="alert">{store.error}</p>{/if}

  <div>
    <button class="rounded-md bg-accent px-5 py-2 text-base font-medium text-accent-fg" disabled={store.busy || !prompt.trim()} type="submit">
      Start run
    </button>
  </div>
</form>
