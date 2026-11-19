<script lang="ts">
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
      toast.ok(saved.changed ? `Saved as v${saved.version}.` : "Nothing changed.");
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
    {#if error}<Callout tone="err" title="This could not be saved">{error}</Callout>{/if}

    <Card title="Who it is" icon="user">
      <div class="space-y-4">
        <div class="flex items-center gap-4">
          <Avatar emoji={draft.avatar.emoji} color={draft.avatar.color} size="xl" />
          <div class="min-w-0 flex-1 space-y-2">
            <div class="flex flex-wrap gap-1.5" role="group" aria-label="Face">
              {#each emojis as e (e)}
                <button type="button" class="grid size-8 place-items-center rounded-lg text-lg transition hover:bg-hover {spec.avatar.emoji === e ? 'bg-accent-soft ring-2 ring-accent' : ''}" aria-pressed={spec.avatar.emoji === e} aria-label={`Face ${e}`} onclick={() => (spec.avatar.emoji = e)}>{e}</button>
              {/each}
            </div>
            <div class="flex flex-wrap gap-1.5" role="group" aria-label="Colour">
              {#each colors as c (c)}<Chip selected={spec.avatar.color === c} onclick={() => (spec.avatar.color = c)}>{c}</Chip>{/each}
            </div>
          </div>
        </div>
        <Field label="Name" required>{#snippet children(f)}<Input id={f.id} bind:value={spec.name} maxlength={80} />{/snippet}</Field>
        <Field label="What it is for" hint="One sentence, up to 280 characters. It appears in the gallery and to MCP clients.">
          {#snippet children(f)}<Input id={f.id} bind:value={spec.description} maxlength={280} />{/snippet}
        </Field>
      </div>
    </Card>

    <Card title="Instructions" icon="book">
      <Field label="What it is told" hint="Markdown. Say how it works, what it must never do, and the shape of a good answer.">
        {#snippet children(f)}<Textarea id={f.id} rows={14} mono bind:value={spec.instructions} maxlength={50000} />{/snippet}
      </Field>
    </Card>

    <Card title="Model" icon="cpu">
      <div class="space-y-4">
        <Field label="Default model" hint="A provider/model, a route, an alias or demo.">
          {#snippet children(f)}
            <Select id={f.id} bind:value={spec.models.default}>
              {#if !(store.models?.models ?? []).some((m) => m.reference === spec.models.default) && !(store.models?.aliases ?? []).includes(spec.models.default) && !(store.models?.routes ?? []).some((r) => "route:" + r === spec.models.default) && spec.models.default !== "demo"}
                <option value={spec.models.default}>{spec.models.default}</option>
              {/if}
              <option value="demo">demo (built-in, no network)</option>
              {#each store.models?.aliases ?? [] as a (a)}<option value={a}>alias: {a}</option>{/each}
              {#each store.models?.routes ?? [] as r (r)}<option value={"route:" + r}>route: {r}</option>{/each}
              {#each store.models?.models ?? [] as m (m.reference)}<option value={m.reference}>{m.reference}</option>{/each}
            </Select>
          {/snippet}
        </Field>
        <Field label="Models it may use" hint="One per line: provider/model, provider/*, demo or *. Empty means any. A fallback can never leave this list.">
          {#snippet children(f)}<Textarea id={f.id} rows={3} mono bind:value={allowedText} placeholder={"anthropic/*\nollama/*"} />{/snippet}
        </Field>
      </div>
    </Card>

    <Card title="What it may do" icon="shield">
      <div class="space-y-4">
        <Field label="Tools" hint="One per line: a tool id or a prefix ending in *, like memory.search. Anything that changes something still asks you first.">
          {#snippet children(f)}<Textarea id={f.id} rows={3} mono bind:value={toolsText} placeholder={"memory.search\nmath.*"} />{/snippet}
        </Field>
        <Field label="Access" hint="One per line: http:HOST, repo:OWNER/REPO, jira:KEY, db:NAME, code:python, mcp:SERVER, read:PATTERN, write:PATTERN. A run can only narrow this.">
          {#snippet children(f)}<Textarea id={f.id} rows={4} mono bind:value={accessText} placeholder={"http:en.wikipedia.org\ncode:python"} />{/snippet}
        </Field>
      </div>
    </Card>

    <Card title="Limits and memory" icon="sliders">
      <div class="grid gap-4 sm:grid-cols-2">
        <Field label="Most model steps">{#snippet children(f)}<Input id={f.id} type="number" min="1" max="1000" value={String(spec.limits.max_iterations)} oninput={(e) => (spec.limits.max_iterations = num(e))} />{/snippet}</Field>
        <Field label="Most tool calls">{#snippet children(f)}<Input id={f.id} type="number" min="0" max="10000" value={String(spec.limits.max_tool_calls)} oninput={(e) => (spec.limits.max_tool_calls = num(e))} />{/snippet}</Field>
      </div>
      <div class="mt-4 space-y-2.5">
        <Switch bind:checked={spec.agent_memory}>Its own memory: facts only this agent sees (you still approve each one)</Switch>
        <Switch bind:checked={spec.memory.long_term}>Things you asked ZORIX to remember</Switch>
        <Switch bind:checked={spec.memory.conversation}>Earlier messages of the conversation</Switch>
        <Switch checked={spec.memory.knowledge === "auto"} onchange={(e) => (spec.memory.knowledge = (e.target as HTMLInputElement).checked ? "auto" : "off")}>Indexed documents</Switch>
        <Switch checked={spec.memory.operations === "auto"} onchange={(e) => (spec.memory.operations = (e.target as HTMLInputElement).checked ? "auto" : "off")}>Records of past work</Switch>
      </div>
    </Card>
  </div>

  <div class="pointer-events-none sticky bottom-0 z-10 flex justify-center px-8 pb-5">
    <div class="pointer-events-auto flex w-full max-w-3xl items-center gap-3 rounded-2xl border border-line bg-panel/95 p-3 shadow-3 backdrop-blur">
      <div class="min-w-0 flex-1">
        <Input bind:value={note} placeholder={dirty ? "Why this change? (optional note for the history)" : "No changes yet"} aria-label="Note for the version history" disabled={!dirty} />
      </div>
      <Button onclick={reset} disabled={!dirty || saving}>Discard</Button>
      <Button variant="primary" icon="check" loading={saving} disabled={!dirty} onclick={save}>Save as v{detail.agent.current_version + 1}</Button>
    </div>
  </div>
</div>
