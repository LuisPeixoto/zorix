<script lang="ts">
  import type { Snippet } from "svelte";

  import Badge from "./Badge.svelte";
  import CodeBlock from "./CodeBlock.svelte";
  import Icon from "./Icon.svelte";
  import Spinner from "./Spinner.svelte";
  import { cn } from "./cn.ts";
  import type { UiTone } from "./tone.ts";

  export type ToolStatus = "requested" | "awaiting_approval" | "started" | "succeeded" | "failed" | "denied";

  /** One tool call in the flow of a conversation: what was called, with what, and how it went. An approval can sit inside it. */
  let {
    tool,
    status,
    args,
    result,
    error,
    open = $bindable(false),
    children,
  }: { tool: string; status: ToolStatus; args?: unknown; result?: string; error?: string | null; open?: boolean; children?: Snippet } = $props();

  const LOOK: Record<ToolStatus, { tone: UiTone; text: string; icon: string }> = {
    requested: { tone: "muted", text: "Queued", icon: "clock" },
    awaiting_approval: { tone: "warn", text: "Needs you", icon: "shield" },
    started: { tone: "info", text: "Running", icon: "wrench" },
    succeeded: { tone: "ok", text: "Done", icon: "check" },
    failed: { tone: "err", text: "Failed", icon: "x" },
    denied: { tone: "muted", text: "Refused", icon: "lock" },
  };
  const look = $derived(LOOK[status]);
  const argText = $derived(args === undefined || args === null ? "" : JSON.stringify(args, null, 2));
  const summary = $derived(argText.replace(/\s+/g, " ").slice(0, 90));
  const panel = $props.id();
</script>

<div class={cn("overflow-hidden rounded-xl border bg-panel-2", status === "awaiting_approval" ? "border-warn/50" : "border-line")}>
  <button type="button" class="flex w-full items-center gap-2.5 px-3 py-2 text-left transition hover:bg-hover" aria-expanded={open} aria-controls={open ? panel : undefined} onclick={() => (open = !open)}>
    <span class="grid size-6 shrink-0 place-items-center rounded-md bg-panel text-muted shadow-1">
      {#if status === "started"}<Spinner size={13} />{:else}<Icon name={look.icon} size={13} />{/if}
    </span>
    <span class="mono min-w-0 shrink-0 text-sm font-medium">{tool}</span>
    {#if !open && summary}<span class="mono min-w-0 flex-1 truncate text-xs text-faint">{summary}</span>{:else}<span class="flex-1"></span>{/if}
    <Badge tone={look.tone} live={status === "started"}>{look.text}</Badge>
    <Icon name={open ? "chevron-down" : "chevron-right"} size={14} class="text-faint" />
  </button>
  {#if open}
    <div id={panel} class="space-y-2 border-t border-line-soft p-3">
      {#if argText}
        <div>
          <p class="mb-1 text-2xs font-semibold tracking-wider text-faint uppercase">Asked with</p>
          <CodeBlock code={argText} language="json" maxHeight={180} label="Arguments" />
        </div>
      {/if}
      {#if error}
        <p class="rounded-lg border border-err/35 bg-err/8 px-3 py-2 text-sm text-err" role="alert">{error}</p>
      {:else if result}
        <div>
          <p class="mb-1 text-2xs font-semibold tracking-wider text-faint uppercase">Result</p>
          <CodeBlock code={result} language="text" maxHeight={220} label="Result" wrap />
        </div>
      {/if}
    </div>
  {/if}
  {#if children}<div class="border-t border-line-soft p-3">{@render children()}</div>{/if}
</div>
