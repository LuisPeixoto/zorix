<script lang="ts">
  import { onDestroy, onMount } from "svelte";

  import {
    type EmbeddingModel,
    type EmbeddingsState,
    embeddingModels,
    embeddingsInfo,
    embeddingsOff,
    errorText,
    onKnowledge,
    reembed,
    reembedding,
    setEmbeddings,
    testEmbeddings,
  } from "../api.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import { Badge, Button, Callout, Card, Checkbox, Field, Select } from "../ui/index.ts";

  let info = $state<EmbeddingsState | null>(null);
  let models = $state<EmbeddingModel[]>([]);
  let problems = $state<string[]>([]);
  let pick = $state("");
  let cloud = $state(false);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let running = $state(false);
  let stop: (() => void) | null = null;

  async function load() {
    try {
      info = await embeddingsInfo();
      running = await reembedding();
      pick = info.model ?? pick;
      cloud = info.allow_cloud;
    } catch (e) {
      error = errorText(e);
    }
  }

  async function loadModels() {
    try {
      [models, problems] = await embeddingModels();
    } catch (e) {
      error = errorText(e);
    }
  }

  onMount(async () => {
    await load();
    await loadModels();
    stop = await onKnowledge({
      embedded: (d) => {
        running = false;
        if (d.error) toast.err(d.error);
        else toast.ok("Vectors are up to date.");
        void load();
      },
    });
  });
  onDestroy(() => stop?.());

  const chosen = $derived(models.find((m) => m.reference === pick));
  const needsCloud = $derived(!!pick && chosen !== undefined && !chosen.local);
  const percent = $derived(info && info.texts ? Math.round((info.embedded / info.texts) * 100) : 0);

  async function act(work: () => Promise<unknown>, done?: string) {
    busy = true;
    error = null;
    try {
      await work();
      if (done) toast.ok(done);
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
      await load();
    }
  }

  const use = () => act(async () => {
    const size = await setEmbeddings(pick, undefined, cloud);
    toast.ok(`Using ${pick} (${size} dimensions). Press Embed everything to make vectors for what is indexed.`);
  });
  const test = () => act(async () => {
    const [size, ms] = await testEmbeddings();
    toast.ok(`${size} dimensions in ${ms} ms.`);
  });
  const embedAll = () => act(async () => {
    running = await reembed();
  });
</script>

<Card title="Search by meaning" icon="sparkles">
  <p class="text-sm text-muted">
    Keyword search always works. An embeddings model also finds text that means the same with different words. Pick a model that runs on
    this computer, for example <span class="mono">ollama/all-minilm</span>.
  </p>

  {#if info?.model}
    <div class="mt-4 flex flex-wrap items-center gap-2">
      <Badge tone="accent" dot>{info.model}</Badge>
      {#if info.dimensions}<Badge tone="muted">{info.dimensions} dimensions</Badge>{/if}
      <Badge tone={percent === 100 ? "ok" : "warn"} dot>{info.embedded} of {info.texts} texts embedded ({percent}%)</Badge>
      {#if info.allow_cloud}<Badge tone="warn" dot>cloud allowed</Badge>{/if}
    </div>
  {:else}
    <p class="mt-3 text-sm text-muted">Off: keyword search only.</p>
  {/if}

  <div class="mt-4 grid gap-3 sm:grid-cols-[1fr_auto] sm:items-end">
    <Field label="Model">
      {#snippet children(f)}
        <Select id={f.id} bind:value={pick}>
          <option value="">Choose a model…</option>
          {#each models as m (m.reference)}
            <option value={m.reference}>{m.reference}{m.dimensions ? ` · ${m.dimensions} dims` : ""}{m.local ? " · on this computer" : " · cloud"}{m.installed ? "" : " · not installed"}</option>
          {/each}
        </Select>
      {/snippet}
    </Field>
    <Button variant="primary" icon="check" loading={busy} disabled={!pick || (needsCloud && !cloud)} onclick={use}>Use it</Button>
  </div>

  {#if needsCloud}
    <div class="mt-3">
      <Callout tone="warn" icon="cloud" title="This sends your text to a cloud provider">
        To make vectors, the text of your documents and facts, and every question a run asks, is sent to the provider.
        {#snippet actions()}
          <Checkbox bind:checked={cloud}>I understand and allow it</Checkbox>
        {/snippet}
      </Callout>
    </div>
  {/if}
  {#each problems as problem}<p class="mt-2 text-xs text-faint">{problem}</p>{/each}
  {#if models.length === 0 && problems.length === 0}
    <p class="mt-2 text-xs text-faint">No embeddings model found. For a local one, run <span class="mono">ollama pull all-minilm</span>.</p>
  {/if}
  {#if error}<div class="mt-3"><Callout tone="err">{error}</Callout></div>{/if}

  {#if info?.model}
    <div class="mt-4 flex flex-wrap gap-2">
      <Button icon="refresh" disabled={busy} onclick={test}>Test</Button>
      <Button icon="database" loading={running} disabled={busy || running} onclick={embedAll}>Embed everything</Button>
      <Button variant="danger" icon="x" disabled={busy} onclick={() => act(() => embeddingsOff(), "Embeddings are off.")}>Turn off</Button>
    </div>
  {/if}
</Card>
