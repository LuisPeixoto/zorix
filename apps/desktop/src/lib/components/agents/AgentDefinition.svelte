<script lang="ts">
  import { t } from "../../i18n/index.ts";
  import { untrack } from "svelte";

  import { type AgentDetail, type AgentSpec, agentSave, errorText } from "../../api.ts";
  import { agents } from "../../stores/agents.svelte.ts";
  import { store } from "../../stores/runs.svelte.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { Avatar, Button, Callout, Card, Chip, Field, Input, Select, Switch, Textarea } from "../../ui/index.ts";

  let { detail }: { detail: AgentDetail } = $props();

  const clone = (s: AgentSpec): AgentSpec => JSON.parse(JSON.stringify(s));
  let spec = $state<AgentSpec>(untrack(() => clone(detail.version.spec)));
  let note = $state("");
  let saving = $state(false);
  let error = $state<string | null>(null);

  // One entry per line in the text boxes; the spec keeps lists.
  const toLines = (list: string[]) => list.join("\n");
  const fromLines = (text: string) => text.split("\n").map((l) => l.trim()).filter(Boolean);
  let toolsText = $state(untrack(() => toLines(detail.version.spec.tools)));
  let accessText = $state(untrack(() => toLines(detail.version.spec.access)));
  let allowedText = $state(untrack(() => toLines(detail.version.spec.models.allowed)));

  const current = $derived(clone(detail.version.spec));
  const draft = $derived<AgentSpec>({ ...spec, tools: fromLines(toolsText), access: fromLines(accessText), models: { ...spec.models, allowed: fromLines(allowedText) } });
  const dirty = $derived(JSON.stringify(draft) !== JSON.stringify(current));

  const emojis = ["🤖", "🔎", "🧐", "📊", "✍️", "🛠️", "📚", "🧪", "🧭", "💡", "🔒", "🚀"];
  const colors = ["accent", "info", "ok", "warn", "err", "muted"];

  async function save() {
    saving = true;
    error = null;
    try {
      const saved = await agentSave(draft, note);
      toast.ok(saved.changed ? t("agents.agentDefinition.savedAsV", { version: saved.version }) : t("agents.agentDefinition.nothingChanged"));
      note = "";
      await agents.reload();
    } catch (e) {
      error = errorText(e);
    } finally {
      saving = false;
    }
  }

  function reset() {
    spec = clone(detail.version.spec);
    toolsText = toLines(spec.tools);
    accessText = toLines(spec.access);
    allowedText = toLines(spec.models.allowed);
    error = null;
  }
  const num = (e: Event) => Number((e.target as HTMLInputElement).value);
</script>

<div class="h-full overflow-y-auto">
  <div class="mx-auto flex max-w-3xl flex-col gap-5 p-8 pb-28">
    {#if error}<Callout tone="err" title={t("agents.agentDefinition.thisCouldNotBe")}>{error}</Callout>{/if}

    <Card title={t("agents.agentDefinition.whoItIs")} icon="user">
      <div class="space-y-4">
        <div class="flex items-center gap-4">
          <Avatar emoji={draft.avatar.emoji} color={draft.avatar.color} size="xl" />
          <div class="min-w-0 flex-1 space-y-2">
            <div class="flex flex-wrap gap-1.5" role="group" aria-label={t("agents.agentDefinition.face")}>
              {#each emojis as e (e)}
                <button type="button" class="grid size-8 place-items-center rounded-lg text-lg transition hover:bg-hover {spec.avatar.emoji === e ? 'bg-accent-soft ring-2 ring-accent' : ''}" aria-pressed={spec.avatar.emoji === e} aria-label={t("agents.agentDefinition.face2", { e })} onclick={() => (spec.avatar.emoji = e)}>{e}</button>
              {/each}
            </div>
            <div class="flex flex-wrap gap-1.5" role="group" aria-label={t("agents.agentDefinition.colour")}>
              {#each colors as c (c)}<Chip selected={spec.avatar.color === c} onclick={() => (spec.avatar.color = c)}>{c}</Chip>{/each}
            </div>
          </div>
        </div>
        <Field label={t("common.name")} required>{#snippet children(f)}<Input id={f.id} bind:value={spec.name} maxlength={80} />{/snippet}</Field>
        <Field label={t("common.whatItIsFor")} hint={t("agents.agentDefinition.oneSentenceUpTo")}>
          {#snippet children(f)}<Input id={f.id} bind:value={spec.description} maxlength={280} />{/snippet}
        </Field>
      </div>
    </Card>

    <Card title={t("agents.agentDefinition.instructions")} icon="book">
      <Field label={t("agents.agentDefinition.whatItIsTold")} hint={t("agents.agentDefinition.markdownSayHowIt")}>
        {#snippet children(f)}<Textarea id={f.id} rows={14} mono bind:value={spec.instructions} maxlength={50000} />{/snippet}
      </Field>
    </Card>

    <Card title={t("common.model")} icon="cpu">
      <div class="space-y-4">
        <Field label={t("agents.agentDefinition.defaultModel")} hint={t("agents.agentDefinition.aProviderModelA")}>
          {#snippet children(f)}
            <Select id={f.id} bind:value={spec.models.default}>
              {#if !(store.models?.models ?? []).some((m) => m.reference === spec.models.default) && !(store.models?.aliases ?? []).includes(spec.models.default) && !(store.models?.routes ?? []).some((r) => "route:" + r === spec.models.default) && spec.models.default !== "demo"}
                <option value={spec.models.default}>{spec.models.default}</option>
              {/if}
              <option value="demo">{t("agents.agentDefinition.demoBuiltInNo")}</option>
              {#each store.models?.aliases ?? [] as a (a)}<option value={a}>alias: {a}</option>{/each}
              {#each store.models?.routes ?? [] as r (r)}<option value={"route:" + r}>route: {r}</option>{/each}
              {#each store.models?.models ?? [] as m (m.reference)}<option value={m.reference}>{m.reference}</option>{/each}
            </Select>
          {/snippet}
        </Field>
        <Field label={t("agents.agentDefinition.modelsItMayUse")} hint={t("agents.agentDefinition.onePerLineProvider")}>
          {#snippet children(f)}<Textarea id={f.id} rows={3} mono bind:value={allowedText} placeholder={t("agents.agentDefinition.anthropicOllama")} />{/snippet}
        </Field>
      </div>
    </Card>

    <Card title={t("agents.agentDefinition.whatItMayDo")} icon="shield">
      <div class="space-y-4">
        <Field label={t("agents.agentDefinition.tools")} hint={t("agents.agentDefinition.onePerLineA")}>
          {#snippet children(f)}<Textarea id={f.id} rows={3} mono bind:value={toolsText} placeholder={t("agents.agentDefinition.memorySearchMath")} />{/snippet}
        </Field>
        <Field label={t("agents.agentDefinition.access")} hint={t("agents.agentDefinition.onePerLineHttp")}>
          {#snippet children(f)}<Textarea id={f.id} rows={4} mono bind:value={accessText} placeholder={"http:en.wikipedia.org\ncode:python"} />{/snippet}
        </Field>
      </div>
    </Card>

    <Card title={t("agents.agentDefinition.limitsAndMemory")} icon="sliders">
      <div class="grid gap-4 sm:grid-cols-2">
        <Field label={t("agents.agentDefinition.mostModelSteps")}>{#snippet children(f)}<Input id={f.id} type="number" min="1" max="1000" value={String(spec.limits.max_iterations)} oninput={(e) => (spec.limits.max_iterations = num(e))} />{/snippet}</Field>
        <Field label={t("agents.agentDefinition.mostToolCalls")}>{#snippet children(f)}<Input id={f.id} type="number" min="0" max="10000" value={String(spec.limits.max_tool_calls)} oninput={(e) => (spec.limits.max_tool_calls = num(e))} />{/snippet}</Field>
      </div>
      <div class="mt-4 space-y-2.5">
        <Switch bind:checked={spec.agent_memory}>{t("agents.agentDefinition.itsOwnMemoryFacts")}</Switch>
        <Switch bind:checked={spec.memory.long_term}>{t("agents.agentDefinition.thingsYouAskedZorix")}</Switch>
        <Switch bind:checked={spec.memory.conversation}>{t("agents.agentDefinition.earlierMessagesOfThe")}</Switch>
        <Switch checked={spec.memory.knowledge === "auto"} onchange={(e) => (spec.memory.knowledge = (e.target as HTMLInputElement).checked ? "auto" : "off")}>{t("agents.agentDefinition.indexedDocuments")}</Switch>
        <Switch checked={spec.memory.operations === "auto"} onchange={(e) => (spec.memory.operations = (e.target as HTMLInputElement).checked ? "auto" : "off")}>{t("agents.agentDefinition.recordsOfPastWork")}</Switch>
      </div>
    </Card>
  </div>

  <div class="pointer-events-none sticky bottom-0 z-10 flex justify-center px-8 pb-5">
    <div class="pointer-events-auto flex w-full max-w-3xl items-center gap-3 rounded-2xl border border-line bg-panel/95 p-3 shadow-3 backdrop-blur">
      <div class="min-w-0 flex-1">
        <Input bind:value={note} placeholder={dirty ? t("agents.agentDefinition.whyThisChangeOptional") : t("agents.agentDefinition.noChangesYet")} aria-label={t("agents.agentDefinition.noteForTheVersion")} disabled={!dirty} />
      </div>
      <Button onclick={reset} disabled={!dirty || saving}>{t("agents.agentDefinition.discard")}</Button>
      <Button variant="primary" icon="check" loading={saving} disabled={!dirty} onclick={save}>{t("agents.agentDefinition.saveAsV", { value: detail.agent.current_version + 1 })}</Button>
    </div>
  </div>
</div>
