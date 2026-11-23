<script lang="ts">
  import { fmt, t } from "../../i18n/index.ts";
  import { type WfWait, type WorkflowSpec, errorText, workflowDecide } from "../../api.ts";
  import { json } from "../../format.ts";
  import { store } from "../../stores/runs.svelte.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { Button, Callout, CodeBlock, Field, Icon, Input, Textarea } from "../../ui/index.ts";
  import { ms } from "../../workflow.ts";

  /**
   * A workflow stopped for a person (design system: ApprovalCard). It shows exactly what the person is asked and what the step
   * chose to show them. The answer names the fingerprint of that, so it cannot be applied to anything else.
   */
  let { wait, spec, onanswered }: { wait: WfWait; spec: WorkflowSpec; onanswered: () => void } = $props();

  let note = $state("");
  let answer = $state("");
  let busy = $state(false);

  const shown = $derived(Object.entries(wait.shown ?? {}).filter(([key]) => key !== "tool" && key !== "arguments" && key !== "interrupted"));
  const call = $derived(wait.kind === "action" ? { tool: String(wait.shown.tool ?? ""), arguments: wait.shown.arguments, interrupted: wait.shown.interrupted === true } : null);
  const left = $derived(wait.due_at ? Math.max(0, wait.due_at - store.now) : null);

  function label(path: string): string {
    const step = /^steps\.([^.]+)\.output/.exec(path);
    if (step) return spec.steps.find((s) => s.id === step[1])?.title ?? step[1];
    if (path === "trigger.payload") return t("workflows.workflowWaitCard.whatStartedIt");
    if (path === "run.cost_micros") return t("workflows.workflowWaitCard.costSoFar");
    return path;
  }
  function text(path: string, value: unknown): string {
    if (path === "run.cost_micros" && typeof value === "number") return fmt.usd2(value);
    return typeof value === "string" ? value : "";
  }

  async function decide(verdict: "approve" | "refuse" | "answer") {
    busy = true;
    try {
      await workflowDecide(wait.id, wait.fingerprint, verdict, verdict === "answer" ? answer : undefined, note);
      toast.ok(verdict === "refuse" ? t("workflows.workflowWaitCard.refused") : verdict === "answer" ? t("workflows.workflowWaitCard.answerSent") : t("workflows.workflowWaitCard.approved"));
      onanswered();
    } catch (e) {
      toast.err(errorText(e));
      onanswered();
    } finally {
      busy = false;
    }
  }

  const ASKS = {
    approval: t("workflows.workflowWaitCard.asksApproval"),
    question: t("workflows.workflowWaitCard.asksQuestion"),
    action: t("workflows.workflowWaitCard.asksAction"),
    timer: "",
  } as const;
</script>

<div class="overflow-hidden rounded-2xl border border-warn/45 bg-linear-to-b from-warn/9 to-panel to-60% shadow-raised ring-4 ring-warn/7">
  <div class="flex items-start gap-3 px-4 pt-4 pb-2.5">
    <span class="grid size-8.5 shrink-0 place-items-center rounded-xl bg-warn/15 text-warn"><Icon name={wait.kind === "question" ? "message" : wait.kind === "action" ? "send" : "shield-check"} size={17} /></span>
    <div class="min-w-0 flex-1">
      <p class="text-md font-semibold">{wait.message}</p>
      <p class="mt-0.5 text-sm text-muted">{t("workflows.workflowWaitCard.theStep", { value: ASKS[wait.kind] })} <b class="font-medium text-fg">{wait.title}</b>.</p>
    </div>
  </div>

  {#if call}
    <div class="space-y-2.5 px-4">
      {#if call.interrupted}
        <Callout tone="warn" title={t("workflows.workflowWaitCard.itMayAlreadyHave")}>{t("workflows.workflowWaitCard.zorixStoppedWhileThis")}</Callout>
      {/if}
      <p class="text-xs font-medium text-muted">{t("workflows.workflowWaitCard.nothingIsSentUntil")}</p>
      <div class="rounded-lg border border-line bg-panel px-3 py-2"><span class="mono text-sm font-semibold">{call.tool}</span></div>
      <CodeBlock code={json(call.arguments)} language="json" maxHeight={200} label={t("workflows.workflowWaitCard.whatWouldBeSent")} />
    </div>
  {:else if shown.length > 0}
    <dl class="mx-4 space-y-2.5 rounded-xl border border-line bg-panel p-3">
      {#each shown as [path, value] (path)}
        <div>
          <dt class="text-xs font-medium text-muted">{label(path)}</dt>
          <dd class="selectable mt-0.5 text-base whitespace-pre-wrap">
            {#if text(path, value)}{text(path, value)}{:else}<CodeBlock code={json(value)} language="json" maxHeight={140} label={label(path)} />{/if}
          </dd>
        </div>
      {/each}
    </dl>
  {/if}

  {#if wait.kind === "question"}
    <div class="px-4 pt-3">
      <Field label={t("workflows.workflowWaitCard.yourAnswer")}>
        {#snippet children(f)}<Textarea id={f.id} rows={3} bind:value={answer} maxlength={4000} />{/snippet}
      </Field>
    </div>
  {/if}

  <div class="px-4 pt-3">
    <Field label={t("workflows.workflowWaitCard.noteOptional")}>
      {#snippet children(f)}<Input id={f.id} bind:value={note} maxlength={300} />{/snippet}
    </Field>
  </div>

  {#if left !== null}
    <p class="px-4 pt-2.5 text-xs text-muted">{t("workflows.workflowWaitCard.timesOutIn")} <span class="tnum">{ms(Math.max(left, 60_000))}</span>{t("workflows.workflowWaitCard.then")} {wait.then === "continue" ? t("workflows.workflowWaitCard.goesOn") : "refuses"}.</p>
  {/if}

  <div class="flex flex-wrap items-center gap-2 px-4 pt-3 pb-3.5">
    <span class="mono mr-auto inline-flex items-center gap-1.5 text-2xs text-faint"><Icon name="lock" size={12} />sha256 {wait.fingerprint.slice(0, 12)}…</span>
    <Button disabled={busy} onclick={() => void decide("refuse")}>{t("common.refuse")}</Button>
    {#if wait.kind === "question"}
      <Button variant="primary" icon="send" disabled={busy || !answer.trim()} onclick={() => void decide("answer")}>{t("workflows.workflowWaitCard.sendAnswer")}</Button>
    {:else}
      <Button variant="primary" icon="check" disabled={busy} onclick={() => void decide("approve")}>{wait.kind === "action" ? t("workflows.workflowWaitCard.sendIt") : t("common.approve")}</Button>
    {/if}
  </div>
</div>
