<script lang="ts">
  import { t } from "../i18n/index.ts";
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
  import { store } from "../stores/runs.svelte.ts";
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
    // For screenshots: the card sits below the providers, so a review can ask to see it.
    if (store.startTab === "embeddings") setTimeout(() => document.getElementById("embeddings")?.scrollIntoView(), 600);
    await load();
    await loadModels();
    stop = await onKnowledge({
      embedded: (d) => {
        running = false;
        if (d.error) toast.err(d.error);
        else toast.ok(t("embeddings.vectorsAreUpTo"));
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
    toast.ok(t("embeddings.usingDimensionsPressEmbed", { pick, size }));
  });
  const test = () => act(async () => {
    const [size, ms] = await testEmbeddings();
    toast.ok(t("embeddings.dimensionsInMs", { size, ms }));
  });
  const embedAll = () => act(async () => {
    running = await reembed();
  });
</script>

<div id="embeddings">
<Card title={t("embeddings.searchByMeaning")} icon="sparkles">
  <p class="text-sm text-muted">
    {t("embeddings.keywordSearchAlwaysWorks")} <span class="mono">ollama/all-minilm</span>.
  </p>

  {#if info?.model}
    <div class="mt-4 flex flex-wrap items-center gap-2">
      <Badge tone="accent" dot>{info.model}</Badge>
      {#if info.dimensions}<Badge tone="muted">{info.dimensions} dimensions</Badge>{/if}
      <Badge tone={percent === 100 ? "ok" : "warn"} dot>{t("embeddings.ofTextsEmbedded", { embedded: info.embedded, texts: info.texts, percent })}</Badge>
      {#if info.allow_cloud}<Badge tone="warn" dot>{t("embeddings.cloudAllowed")}</Badge>{/if}
    </div>
  {:else}
    <p class="mt-3 text-sm text-muted">{t("embeddings.offKeywordSearchOnly")}</p>
  {/if}

  <div class="mt-4 grid gap-3 sm:grid-cols-[1fr_auto] sm:items-end">
    <Field label={t("common.model")}>
      {#snippet children(f)}
        <Select id={f.id} bind:value={pick}>
          <option value="">{t("embeddings.chooseAModel")}</option>
          {#if info?.model && !models.some((m) => m.reference === info?.model)}
            <option value={info.model}>{t("embeddings.inUse", { model: info.model })}</option>
          {/if}
          {#each models as m (m.reference)}
            <option value={m.reference}>{m.reference}{m.dimensions ? ` · ${m.dimensions} dims` : ""}{m.local ? t("embeddings.onThisComputer") : t("embeddings.cloud")}{m.installed ? "" : t("embeddings.notInstalled")}</option>
          {/each}
        </Select>
      {/snippet}
    </Field>
    <Button variant="primary" icon="check" loading={busy} disabled={!pick || (needsCloud && !cloud) || (pick === info?.model && cloud === info?.allow_cloud)} onclick={use}>{t("embeddings.useIt")}</Button>
  </div>

  {#if needsCloud}
    <div class="mt-3">
      <Callout tone="warn" icon="cloud" title={t("embeddings.thisSendsYourText")}>
        {t("embeddings.toMakeVectorsThe")}
        {#snippet actions()}
          <Checkbox bind:checked={cloud}>{t("embeddings.iUnderstandAndAllow")}</Checkbox>
        {/snippet}
      </Callout>
    </div>
  {/if}
  {#each problems as problem}<p class="mt-2 text-xs text-faint">{problem}</p>{/each}
  {#if models.length === 0 && problems.length === 0}
    <p class="mt-2 text-xs text-faint">{t("embeddings.noEmbeddingsModelFound")} <span class="mono">{t("embeddings.ollamaPullAllMinilm")}</span>.</p>
  {/if}
  {#if error}<div class="mt-3"><Callout tone="err">{error}</Callout></div>{/if}

  {#if info?.model}
    <div class="mt-4 flex flex-wrap gap-2">
      <Button icon="refresh" disabled={busy} onclick={test}>{t("common.test")}</Button>
      <Button icon="database" loading={running} disabled={busy || running} onclick={embedAll}>{t("embeddings.embedEverything")}</Button>
      <Button variant="danger" icon="x" disabled={busy} onclick={() => act(() => embeddingsOff(), t("embeddings.areOff"))}>{t("embeddings.turnOff")}</Button>
    </div>
  {/if}
</Card>
</div>
