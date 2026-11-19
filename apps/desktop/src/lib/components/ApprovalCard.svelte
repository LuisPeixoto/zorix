<script lang="ts">
  import type { Approval } from "../api.ts";
  import { json } from "../format.ts";
  import { integrations } from "../stores/integrations.svelte.ts";
  import { store } from "../stores/runs.svelte.ts";
  import { Badge, BrandLogo, Button, Callout, CodeBlock, Field, Icon, Input } from "../ui/index.ts";
  import { actionWords, approvalVariant, splitMcpTool, splitService } from "./approvalVariant.ts";

  let { approval }: { approval: Approval } = $props();
  let note = $state("");

  const variant = $derived(approvalVariant(approval.tool));
  const args = $derived((approval.arguments ?? {}) as Record<string, unknown>);
  const text = (key: string) => (typeof args[key] === "string" ? (args[key] as string) : "");

  // What the engine said when it asked: who made the tool, a sentence about this call, and the pinned definition.
  const asked = $derived(store.events.find((e) => e.kind === "execution.waiting_approval" && e.payload.approval_id === approval.id));
  const summary = $derived(typeof asked?.payload.summary === "string" ? (asked.payload.summary as string) : "");
  const hash = $derived(typeof asked?.payload.definition_hash === "string" ? (asked.payload.definition_hash as string) : "");

  // A proposal to remember something gets a layout of its own: what would be remembered, where, and what it would replace.
  const replaces = $derived(Array.isArray(args.replaces) ? (args.replaces as string[]) : []);
  const scopeText = $derived(typeof args.scope === "string" ? args.scope : "global");
  const subject = $derived(typeof args.subject === "string" ? args.subject : "");
  const affected = $derived(
    store.memory.filter(
      (m) => m.status === "active" && (replaces.some((r) => r.replace(/^m:/, "") === m.id.replace(/^m:/, "")) || (subject && m.subject === subject && m.scope === scopeText)),
    ),
  );
  $effect(() => {
    if (variant === "remember") void store.refreshMemory(false);
  });

  const mcp = $derived(splitMcpTool(approval.tool));
  const server = $derived(mcp ? integrations.servers.find((s) => s.name === mcp.server) : undefined);
  const service = $derived(splitService(approval.tool));
  const decide = (yes: boolean) => store.decide(approval.id, approval.call_hash, yes, note);
  const body = $derived(text("body") || text("text") || text("message"));
  const httpBody = $derived(args.json !== undefined ? JSON.stringify(args.json, null, 2) : text("body"));
</script>

{#snippet answer(yes: string)}
  <div class="mt-3">
    <Field label="Note (optional; the model reads it if you refuse)">
      {#snippet children(f)}<Input id={f.id} bind:value={note} maxlength={300} />{/snippet}
    </Field>
  </div>
  <p class="mt-2 text-xs text-muted">Your answer applies to exactly this call (fingerprint <span class="mono">{approval.call_hash.slice(0, 12)}…</span>).</p>
{/snippet}

{#snippet buttons(yes: string)}
  <Button variant="primary" icon="check" disabled={store.busy} onclick={() => decide(true)}>{yes}</Button>
  <Button icon="x" disabled={store.busy} onclick={() => decide(false)}>Refuse</Button>
{/snippet}

{#if variant === "remember"}
  <Callout tone="warn" icon="book" title="A run wants to remember something">
    <p>If you approve, this is saved and shown to future runs. Nothing is saved until you do.</p>
    <blockquote class="selectable mt-3 rounded-lg border border-line bg-panel p-3 text-md leading-relaxed text-fg">{String(args.content ?? "")}</blockquote>
    <div class="mt-2.5 flex flex-wrap gap-1.5">
      <Badge tone="muted">{String(args.kind ?? "fact")}</Badge>
      <Badge tone="accent">{scopeText === "global" ? "Everywhere" : scopeText}</Badge>
      {#if subject}<Badge tone="muted">about: {subject}</Badge>{/if}
      {#if args.valid_until}<Badge tone="muted">until {String(args.valid_until)}</Badge>{/if}
    </div>
    {#if affected.length}
      <div class="mt-3 rounded-lg border border-warn/40 bg-warn/8 p-3">
        <p class="text-sm font-medium">This would replace:</p>
        {#each affected as old (old.id)}<p class="selectable mt-1 text-sm">{old.content}</p>{/each}
      </div>
    {/if}
    <p class="mt-3 text-xs text-muted">You cannot edit a proposal before approving it. Refuse it with a note, or approve it and edit it under Memory.</p>
    {@render answer("Remember it")}
    {#snippet actions()}{@render buttons("Remember it")}{/snippet}
  </Callout>
{:else}
  <Callout tone="warn" icon="shield" title={variant === "mcp" ? "A server's tool wants to run" : variant === "db" ? "A run wants to change a database" : variant === "chat" ? "A run wants to send a message" : variant === "code" ? "A run wants to run code with files" : "Approval needed"}>
    <div class="mt-2.5">
    {#if variant === "mcp" && mcp}
      <div class="flex items-center gap-3 rounded-lg border border-line bg-panel p-3">
        <BrandLogo name={server?.logo ?? (server?.kind === "http" ? "globe" : "server")} size={36} />
        <div class="min-w-0 flex-1">
          <p class="text-md font-medium">{mcp.server} <span class="text-muted">›</span> <span class="mono">{mcp.tool}</span></p>
          <p class="text-xs text-muted">
            {server ? (server.kind === "stdio" ? "A program on this computer" : "A remote server") : "An MCP server"}. Its tool and description were approved by you{hash ? ` (definition ${hash.slice(0, 10)})` : ""}.
          </p>
        </div>
        <Badge tone="muted">third party</Badge>
      </div>
      {#if summary}<p class="mt-3 text-base text-fg">{summary}</p>{/if}
      <p class="mt-3 mb-1 text-xs font-medium text-muted">Arguments</p>
      <CodeBlock code={json(approval.arguments)} language="json" maxHeight={220} label="Arguments" />
      <p class="mt-2 text-xs text-muted">What the server returns is treated as data, never as instructions.</p>
    {:else if variant === "http" || variant === "webhook"}
      <p>The run wants to {variant === "webhook" ? "send a signed webhook" : "send a web request"}.</p>
      <div class="mt-3 flex items-center gap-2 rounded-lg border border-line bg-panel px-3 py-2">
        {#if variant === "http"}<Badge tone={text("method") === "GET" ? "ok" : "warn"}>{text("method") || "GET"}</Badge>{:else}<Badge tone="accent">POST</Badge>{/if}
        <span class="mono selectable min-w-0 flex-1 truncate text-sm" title={text("url") || text("connection")}>{variant === "http" ? text("url") : `${text("connection")} · ${text("event")}`}</span>
      </div>
      {#if summary}<p class="mt-2 text-sm text-muted">{summary}</p>{/if}
      {#if variant === "http" && httpBody}
        <p class="mt-3 mb-1 text-xs font-medium text-muted">Body</p>
        <CodeBlock code={httpBody} language={args.json !== undefined ? "json" : "text"} maxHeight={200} wrap label="Request body" />
      {:else if variant === "webhook"}
        <p class="mt-3 mb-1 text-xs font-medium text-muted">Payload</p>
        <CodeBlock code={JSON.stringify(args.payload ?? {}, null, 2)} language="json" maxHeight={200} label="Payload" />
      {/if}
    {:else if variant === "git" || variant === "jira"}
      <p>The run wants to <strong class="text-fg">{actionWords(service.action)}</strong> on {service.service === "github" ? "GitHub" : service.service === "gitlab" ? "GitLab" : "Jira"}.</p>
      <div class="mt-3 flex flex-wrap items-center gap-2 rounded-lg border border-line bg-panel px-3 py-2">
        <Icon name={variant === "git" ? "git-branch" : "list"} size={15} class="text-muted" />
        <span class="mono selectable text-sm">{text("repo") || text("project") || text("key") || ""}{args.number ? ` #${args.number}` : ""}</span>
        {#if text("state")}<Badge tone="muted">{text("state")}</Badge>{/if}
        {#if text("transition")}<Badge tone="accent">{text("transition")}</Badge>{/if}
      </div>
      {#if text("title") || text("summary")}<p class="selectable mt-3 text-md font-medium">{text("title") || text("summary")}</p>{/if}
      {#if body}<blockquote class="selectable mt-2 max-h-52 overflow-auto rounded-lg border border-line bg-panel p-3 text-base leading-relaxed whitespace-pre-wrap text-fg">{body}</blockquote>{/if}
      {#if !text("title") && !body && !text("summary")}<CodeBlock code={json(approval.arguments)} language="json" maxHeight={200} label="Arguments" class="mt-3" />{/if}
    {:else if variant === "chat"}
      <p>The run wants to post in <strong class="text-fg">{service.service === "slack" ? "Slack" : "Discord"}</strong> channel <span class="mono text-fg">{text("channel")}</span>.</p>
      <div class="mt-3 rounded-2xl rounded-tl-sm border border-line bg-panel p-3.5 shadow-1">
        <p class="selectable text-md leading-relaxed whitespace-pre-wrap">{text("text") || text("content")}</p>
      </div>
      <p class="mt-1.5 text-xs text-muted">Mentions of everyone are refused automatically. Nothing is sent until you approve.</p>
    {:else if variant === "db"}
      <p>The run wants to run this statement on <strong class="mono text-fg">{text("connection")}</strong>. Queries cannot change data; this one can.</p>
      <CodeBlock code={text("sql")} language="sql" maxHeight={240} wrap label="SQL statement" class="mt-3" />
      <Callout tone="err" icon="alert" class="mt-3">It runs in one transaction and is rolled back if it fails, but a successful change is real.</Callout>
    {:else if variant === "code"}
      <p>The run wants to run <strong class="text-fg">{text("language") === "python" ? "Python" : "JavaScript"}</strong> in the sandbox with access to your workspace files (read-only). There is no network.</p>
      <CodeBlock code={text("code")} language={text("language") === "python" ? "py" : "js"} maxHeight={260} label="Program" class="mt-3" />
    {:else}
      <p>The run wants to use <code class="mono text-fg">{approval.tool}</code>. Nothing has happened yet.</p>
      {#if summary}<p class="mt-1 text-fg">{summary}</p>{/if}
      <p class="mt-1">Why you are asked: {approval.reason}</p>
      <CodeBlock code={json(approval.arguments)} language="json" maxHeight={200} label="Arguments" class="mt-3" />
    {/if}
    {#if variant !== "generic" && variant !== "mcp"}<p class="mt-3 text-xs text-muted">Why you are asked: {approval.reason}</p>{/if}
    </div>
    {@render answer("Approve")}
    {#snippet actions()}{@render buttons("Approve")}{/snippet}
  </Callout>
{/if}
