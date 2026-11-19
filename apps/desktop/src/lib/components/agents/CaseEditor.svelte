<script lang="ts">
  import type { EvalCase, EvalCheck } from "../../api.ts";
  import { Button, Field, IconButton, Input, Modal, Select, Switch, Textarea } from "../../ui/index.ts";

  let { open = $bindable(false), initial, onsave }: { open: boolean; initial: EvalCase | null; onsave: (c: EvalCase) => Promise<string | null> } = $props();

  type Kind = "contains" | "not_contains" | "regex" | "tool_used" | "tool_not_used" | "max_steps" | "max_tool_calls";
  type Row = { kind: Kind; value: string };
  const KINDS: { value: Kind; label: string; hint: string }[] = [
    { value: "contains", label: "Answer contains", hint: "text" },
    { value: "not_contains", label: "Answer does not contain", hint: "text" },
    { value: "regex", label: "Answer matches", hint: "regular expression" },
    { value: "tool_used", label: "Used the tool", hint: "tool id, or a prefix ending in *" },
    { value: "tool_not_used", label: "Never used the tool", hint: "tool id, or a prefix ending in *" },
    { value: "max_steps", label: "At most this many steps", hint: "number" },
    { value: "max_tool_calls", label: "At most this many tool calls", hint: "number" },
  ];

  let name = $state("");
  let prompt = $state("");
  let tags = $state("");
  let rows = $state<Row[]>([]);
  let judging = $state(false);
  let rubric = $state("");
  let judgeModel = $state("default");
  let passScore = $state("4");
  let error = $state<string | null>(null);
  let saving = $state(false);

  const toRow = (c: EvalCheck): Row | null => {
    switch (c.kind) {
      case "contains":
      case "not_contains":
        return { kind: c.kind, value: c.text };
      case "regex":
        return { kind: "regex", value: c.pattern };
      case "tool_used":
      case "tool_not_used":
        return { kind: c.kind, value: c.pattern };
      case "max_steps":
        return { kind: "max_steps", value: String(c.steps) };
      case "max_tool_calls":
        return { kind: "max_tool_calls", value: String(c.calls) };
      default:
        return null;
    }
  };
  const toCheck = (r: Row): EvalCheck => {
    switch (r.kind) {
      case "contains":
      case "not_contains":
        return { kind: r.kind, text: r.value, ignore_case: false };
      case "regex":
        return { kind: "regex", pattern: r.value };
      case "tool_used":
        return { kind: "tool_used", pattern: r.value, min: 1 };
      case "tool_not_used":
        return { kind: "tool_not_used", pattern: r.value };
      case "max_steps":
        return { kind: "max_steps", steps: Number(r.value) };
      default:
        return { kind: "max_tool_calls", calls: Number(r.value) };
    }
  };

  // Load the case being edited each time the dialog opens.
  $effect(() => {
    if (!open) return;
    name = initial?.name ?? "";
    prompt = initial?.prompt ?? "";
    tags = (initial?.tags ?? []).join(", ");
    rows = (initial?.checks ?? []).map(toRow).filter((r): r is Row => r !== null);
    if (!initial) rows = [{ kind: "contains", value: "" }];
    judging = !!initial?.judge;
    rubric = initial?.judge?.rubric ?? "";
    judgeModel = initial?.judge?.model ?? "default";
    passScore = String(initial?.judge?.pass_score ?? 4);
    error = null;
  });

  async function save() {
    saving = true;
    error = null;
    const kept = initial?.checks.filter((c) => !toRow(c)) ?? [];
    const result = await onsave({
      name: name.trim(),
      prompt,
      input: initial?.input,
      tags: tags.split(",").map((t) => t.trim()).filter(Boolean),
      checks: [...rows.filter((r) => r.value.trim()).map(toCheck), ...kept],
      judge: judging ? { rubric, model: judgeModel.trim() || "default", pass_score: Number(passScore) } : null,
      approve: initial?.approve ?? [],
      allow_writes: initial?.allow_writes ?? false,
    });
    saving = false;
    if (result) error = result;
    else open = false;
  }
</script>

<Modal bind:open title={initial ? "Edit case" : "New case"} icon="flask" size="lg">
  <div class="space-y-4">
    <Field label="Name" required error={error?.includes("name") ? error : null}>{#snippet children(f)}<Input id={f.id} bind:value={name} maxlength={80} placeholder="cites-its-sources" disabled={!!initial} />{/snippet}</Field>
    <Field label="What to ask" required>{#snippet children(f)}<Textarea id={f.id} rows={4} bind:value={prompt} placeholder="What is the boiling point of water? Give a source." />{/snippet}</Field>
    <Field label="Tags" hint="Comma separated. Run only the cases with a tag, such as smoke.">{#snippet children(f)}<Input id={f.id} bind:value={tags} placeholder="smoke, citations" />{/snippet}</Field>

    <fieldset class="space-y-2">
      <legend class="mb-1 text-sm font-medium">The answer must meet</legend>
      {#each rows as row, i (i)}
        <div class="flex items-center gap-2">
          <Select bind:value={row.kind} class="w-60 shrink-0" aria-label="Kind of check">{#each KINDS as k (k.value)}<option value={k.value}>{k.label}</option>{/each}</Select>
          <Input class="min-w-0 flex-1" bind:value={row.value} aria-label="Value" placeholder={KINDS.find((k) => k.value === row.kind)?.hint} mono={row.kind === "regex" || row.kind.startsWith("tool")} />
          <IconButton icon="x" label="Remove this check" onclick={() => (rows = rows.filter((_, k) => k !== i))} />
        </div>
      {/each}
      <Button size="sm" icon="plus" onclick={() => (rows = [...rows, { kind: "contains", value: "" }])}>Add a check</Button>
    </fieldset>

    <div class="space-y-3 rounded-xl border border-line bg-panel-2 p-3">
      <Switch bind:checked={judging}>Also ask a model to grade the answer</Switch>
      {#if judging}
        <Field label="What a good answer is like" hint="The grader sees your rubric and the answer, and scores it from 1 to 5.">{#snippet children(f)}<Textarea id={f.id} rows={3} bind:value={rubric} placeholder="The answer is correct, short, and names its source." />{/snippet}</Field>
        <div class="grid gap-3 sm:grid-cols-2">
          <Field label="Grader model">{#snippet children(f)}<Input id={f.id} bind:value={judgeModel} mono />{/snippet}</Field>
          <Field label="Passes from">{#snippet children(f)}<Select id={f.id} bind:value={passScore}>{#each [1, 2, 3, 4, 5] as n (n)}<option value={String(n)}>{n} out of 5</option>{/each}</Select>{/snippet}</Field>
        </div>
      {/if}
    </div>
    {#if error}<p class="text-sm text-err" role="alert">{error}</p>{/if}
  </div>
  {#snippet footer()}
    <Button onclick={() => (open = false)}>Cancel</Button>
    <Button variant="primary" icon="check" loading={saving} disabled={!name.trim() || !prompt.trim()} onclick={save}>Save case</Button>
  {/snippet}
</Modal>
