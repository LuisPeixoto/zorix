<script lang="ts">
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
    if (path === "trigger.payload") return "What started it";
    if (path === "run.cost_micros") return "Cost so far";
    return path;
  }
  function text(path: string, value: unknown): string {
    if (path === "run.cost_micros" && typeof value === "number") return `$${(value / 1_000_000).toFixed(2)}`;
    return typeof value === "string" ? value : "";
  }

  async function decide(verdict: "approve" | "refuse" | "answer") {
    busy = true;
    try {
      await workflowDecide(wait.id, wait.fingerprint, verdict, verdict === "answer" ? answer : undefined, note);
      toast.ok(verdict === "refuse" ? "Refused." : verdict === "answer" ? "Answer sent." : "Approved.");
      onanswered();
    } catch (e) {
      toast.err(errorText(e));
      onanswered();
    } finally {
      busy = false;
    }
  }

  const ASKS = { approval: "A person approves", question: "A person answers", action: "This asks first", timer: "" } as const;
</script>

<div class="overflow-hidden rounded-2xl border border-warn/45 bg-linear-to-b from-warn/9 to-panel to-60% shadow-raised ring-4 ring-warn/7">
  <div class="flex items-start gap-3 px-4 pt-4 pb-2.5">
    <span class="grid size-8.5 shrink-0 place-items-center rounded-xl bg-warn/15 text-warn"><Icon name={wait.kind === "question" ? "message" : wait.kind === "action" ? "send" : "shield-check"} size={17} /></span>
    <div class="min-w-0 flex-1">
      <p class="text-md font-semibold">{wait.message}</p>
      <p class="mt-0.5 text-sm text-muted">{ASKS[wait.kind]} · the step <b class="font-medium text-fg">{wait.title}</b>.</p>
    </div>
  </div>

  {#if call}
    <div class="space-y-2.5 px-4">
      {#if call.interrupted}
        <Callout tone="warn" title="It may already have happened">ZORIX stopped while this was running, so it cannot tell whether it went out. Only run it again if sending twice does no harm.</Callout>
      {/if}
      <p class="text-xs font-medium text-muted">Nothing is sent until you approve. If you do, this is what goes out:</p>
      <div class="rounded-lg border border-line bg-panel px-3 py-2"><span class="mono text-sm font-semibold">{call.tool}</span></div>
      <CodeBlock code={json(call.arguments)} language="json" maxHeight={200} label="What would be sent" />
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
      <Field label="Your answer">
        {#snippet children(f)}<Textarea id={f.id} rows={3} bind:value={answer} maxlength={4000} />{/snippet}
      </Field>
    </div>
  {/if}

  <div class="px-4 pt-3">
    <Field label="Note (optional)">
      {#snippet children(f)}<Input id={f.id} bind:value={note} maxlength={300} />{/snippet}
    </Field>
  </div>

  {#if left !== null}
    <p class="px-4 pt-2.5 text-xs text-muted">Times out in <span class="tnum">{ms(Math.max(left, 60_000))}</span>, then {wait.then === "continue" ? "goes on" : "refuses"}.</p>
  {/if}

  <div class="flex flex-wrap items-center gap-2 px-4 pt-3 pb-3.5">
    <span class="mono mr-auto inline-flex items-center gap-1.5 text-2xs text-faint"><Icon name="lock" size={12} />sha256 {wait.fingerprint.slice(0, 12)}…</span>
    <Button disabled={busy} onclick={() => void decide("refuse")}>Refuse</Button>
    {#if wait.kind === "question"}
      <Button variant="primary" icon="send" disabled={busy || !answer.trim()} onclick={() => void decide("answer")}>Send answer</Button>
    {:else}
      <Button variant="primary" icon="check" disabled={busy} onclick={() => void decide("approve")}>{wait.kind === "action" ? "Send it" : "Approve"}</Button>
    {/if}
  </div>
</div>
