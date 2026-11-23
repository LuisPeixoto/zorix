<script lang="ts">
  import { t as tr } from "../../i18n/index.ts";
  import type { EvalCase, EvalCheck } from "../../api.ts";
  import { Button, Field, IconButton, Input, Modal, Select, Switch, Textarea } from "../../ui/index.ts";

  let { open = $bindable(false), initial, onsave }: { open: boolean; initial: EvalCase | null; onsave: (c: EvalCase) => Promise<string | null> } = $props();

  type Kind = "contains" | "not_contains" | "regex" | "tool_used" | "tool_not_used" | "max_steps" | "max_tool_calls";
  type Row = { kind: Kind; value: string };
  const KINDS: { value: Kind; label: string; hint: string }[] = [
    { value: "contains", label: tr("agents.caseEditor.answerContains"), hint: "text" },
    { value: "not_contains", label: tr("agents.caseEditor.answerDoesNotContain"), hint: "text" },
    { value: "regex", label: tr("agents.caseEditor.answerMatches"), hint: tr("agents.caseEditor.regularExpression") },
    { value: "tool_used", label: tr("agents.caseEditor.usedTheTool"), hint: tr("agents.caseEditor.toolIdOrA") },
    { value: "tool_not_used", label: tr("agents.caseEditor.neverUsedTheTool"), hint: tr("agents.caseEditor.toolIdOrA") },
    { value: "max_steps", label: tr("agents.caseEditor.atMostThisMany"), hint: "number" },
    { value: "max_tool_calls", label: tr("agents.caseEditor.atMostThisMany2"), hint: "number" },
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

<Modal bind:open title={initial ? tr("agents.caseEditor.editCase") : tr("agents.caseEditor.newCase")} icon="flask" size="lg">
  <div class="space-y-4">
    <Field label={tr("common.name")} required error={error?.includes("name") ? error : null}>{#snippet children(f)}<Input id={f.id} bind:value={name} maxlength={80} placeholder="cites-its-sources" disabled={!!initial} />{/snippet}</Field>
    <Field label={tr("common.whatToAsk")} required>{#snippet children(f)}<Textarea id={f.id} rows={4} bind:value={prompt} placeholder={tr("agents.caseEditor.whatIsTheBoiling")} />{/snippet}</Field>
    <Field label={tr("agents.caseEditor.tags")} hint={tr("agents.caseEditor.commaSeparatedRunOnly")}>{#snippet children(f)}<Input id={f.id} bind:value={tags} placeholder={tr("agents.caseEditor.smokeCitations")} />{/snippet}</Field>

    <fieldset class="space-y-2">
      <legend class="mb-1 text-sm font-medium">{tr("agents.caseEditor.theAnswerMustMeet")}</legend>
      {#each rows as row, i (i)}
        <div class="flex items-center gap-2">
          <Select bind:value={row.kind} class="w-60 shrink-0" aria-label={tr("agents.caseEditor.kindOfCheck")}>{#each KINDS as k (k.value)}<option value={k.value}>{k.label}</option>{/each}</Select>
          <Input class="min-w-0 flex-1" bind:value={row.value} aria-label={tr("agents.caseEditor.value")} placeholder={KINDS.find((k) => k.value === row.kind)?.hint} mono={row.kind === "regex" || row.kind.startsWith("tool")} />
          <IconButton icon="x" label={tr("agents.caseEditor.removeThisCheck")} onclick={() => (rows = rows.filter((_, k) => k !== i))} />
        </div>
      {/each}
      <Button size="sm" icon="plus" onclick={() => (rows = [...rows, { kind: "contains", value: "" }])}>{tr("agents.caseEditor.addACheck")}</Button>
    </fieldset>

    <div class="space-y-3 rounded-xl border border-line bg-panel-2 p-3">
      <Switch bind:checked={judging}>{tr("agents.caseEditor.alsoAskAModel")}</Switch>
      {#if judging}
        <Field label={tr("agents.caseEditor.whatAGoodAnswer")} hint={tr("agents.caseEditor.theGraderSeesYour")}>{#snippet children(f)}<Textarea id={f.id} rows={3} bind:value={rubric} placeholder={tr("agents.caseEditor.theAnswerIsCorrect")} />{/snippet}</Field>
        <div class="grid gap-3 sm:grid-cols-2">
          <Field label={tr("agents.caseEditor.graderModel")}>{#snippet children(f)}<Input id={f.id} bind:value={judgeModel} mono />{/snippet}</Field>
          <Field label={tr("agents.caseEditor.passesFrom")}>{#snippet children(f)}<Select id={f.id} bind:value={passScore}>{#each [1, 2, 3, 4, 5] as n (n)}<option value={String(n)}>{tr("agents.caseEditor.outOf5", { n })}</option>{/each}</Select>{/snippet}</Field>
        </div>
      {/if}
    </div>
    {#if error}<p class="text-sm text-err" role="alert">{error}</p>{/if}
  </div>
  {#snippet footer()}
    <Button onclick={() => (open = false)}>{tr("common.cancel")}</Button>
    <Button variant="primary" icon="check" loading={saving} disabled={!name.trim() || !prompt.trim()} onclick={save}>{tr("agents.caseEditor.saveCase")}</Button>
  {/snippet}
</Modal>
