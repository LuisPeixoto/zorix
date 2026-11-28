<script lang="ts">
  import { untrack } from "svelte";
  import { t } from "../i18n/index.ts";
    import { json } from "../format.ts";
  import { integrations } from "../stores/integrations.svelte.ts";
  import { store } from "../stores/runs.svelte.ts";
  import { Badge, BrandLogo, Button, Callout, CodeBlock, DiffView, Field, Icon, Input, KeyValue } from "../ui/index.ts";
  import { type Approval, type GitPreview, errorText, gitPreview } from "../api.ts";
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

  // A commit or a push is judged by what changes: asked for from the repository, once per approval.
  let preview = $state<GitPreview | null>(null);
  let previewError = $state<string | null>(null);
  const previewFor = $derived(variant === "repository" ? approval.id : null);
  $effect(() => {
    const id = previewFor;
    if (!id) return;
    preview = null;
    previewError = null;
    void gitPreview(untrack(() => approval.execution_id), id)
      .then((found) => {
        if (previewFor === id) preview = found;
      })
      .catch((e) => (previewError = errorText(e)));
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
    <Field label={t("approvalCard.noteOptionalTheModel")}>
      {#snippet children(f)}<Input id={f.id} bind:value={note} maxlength={300} />{/snippet}
    </Field>
  </div>
  <p class="mt-2 text-xs text-muted">{t("approvalCard.yourAnswerAppliesTo")} <span class="mono">{approval.call_hash.slice(0, 12)}…</span>).</p>
{/snippet}

{#snippet buttons(yes: string)}
  <Button variant="primary" icon="check" disabled={store.busy} onclick={() => decide(true)}>{yes}</Button>
  <Button icon="x" disabled={store.busy} onclick={() => decide(false)}>{t("common.refuse")}</Button>
{/snippet}

{#if variant === "remember"}
  <Callout tone="warn" icon="book" title={t("approvalCard.aRunWantsTo")}>
    <p>{t("approvalCard.ifYouApproveThis")}</p>
    <blockquote class="selectable mt-3 rounded-lg border border-line bg-panel p-3 text-md leading-relaxed text-fg">{String(args.content ?? "")}</blockquote>
    <div class="mt-2.5 flex flex-wrap gap-1.5">
      <Badge tone="muted">{String(args.kind ?? "fact")}</Badge>
      <Badge tone="accent">{scopeText === "global" ? t("approvalCard.everywhere") : scopeText}</Badge>
      {#if subject}<Badge tone="muted">about: {subject}</Badge>{/if}
      {#if args.valid_until}<Badge tone="muted">until {String(args.valid_until)}</Badge>{/if}
    </div>
    {#if affected.length}
      <div class="mt-3 rounded-lg border border-warn/40 bg-warn/8 p-3">
        <p class="text-sm font-medium">{t("approvalCard.thisWouldReplace")}</p>
        {#each affected as old (old.id)}<p class="selectable mt-1 text-sm">{old.content}</p>{/each}
      </div>
    {/if}
    <p class="mt-3 text-xs text-muted">{t("approvalCard.youCannotEditA")}</p>
    {@render answer(t("approvalCard.rememberIt"))}
    {#snippet actions()}{@render buttons(t("approvalCard.rememberIt"))}{/snippet}
  </Callout>
{:else}
  <Callout tone="warn" icon="shield" title={variant === "mcp" ? t("approvalCard.aServerSTool") : variant === "db" ? t("approvalCard.aRunWantsTo2") : variant === "chat" ? t("approvalCard.aRunWantsTo3") : variant === "code" ? t("approvalCard.aRunWantsTo4") : t("approvalCard.approvalNeeded")}>
    <div class="mt-2.5">
    {#if variant === "mcp" && mcp}
      <div class="flex items-center gap-3 rounded-lg border border-line bg-panel p-3">
        <BrandLogo name={server?.logo ?? (server?.kind === "http" ? "globe" : "server")} size={36} />
        <div class="min-w-0 flex-1">
          <p class="text-md font-medium">{mcp.server} <span class="text-muted">›</span> <span class="mono">{mcp.tool}</span></p>
          <p class="text-xs text-muted">
            {server ? (server.kind === "stdio" ? t("approvalCard.aProgramOnThis") : t("approvalCard.aRemoteServer")) : t("approvalCard.anMcpServer")}{t("approvalCard.itsToolAndDescription")}{hash ? ` (definition ${hash.slice(0, 10)})` : ""}.
          </p>
        </div>
        <Badge tone="muted">{t("approvalCard.thirdParty")}</Badge>
      </div>
      {#if summary}<p class="mt-3 text-base text-fg">{summary}</p>{/if}
      <p class="mt-3 mb-1 text-xs font-medium text-muted">{t("common.arguments")}</p>
      <CodeBlock code={json(approval.arguments)} language="json" maxHeight={220} label={t("common.arguments")} />
      <p class="mt-2 text-xs text-muted">{t("approvalCard.whatTheServerReturns")}</p>
    {:else if variant === "http" || variant === "webhook"}
      <p>{t("approvalCard.theRunWantsTo")} {variant === "webhook" ? t("approvalCard.sendASignedWebhook") : t("approvalCard.sendAWebRequest")}.</p>
      <div class="mt-3 flex items-center gap-2 rounded-lg border border-line bg-panel px-3 py-2">
        {#if variant === "http"}<Badge tone={text("method") === "GET" ? "ok" : "warn"}>{text("method") || "GET"}</Badge>{:else}<Badge tone="accent">POST</Badge>{/if}
        <span class="mono selectable min-w-0 flex-1 truncate text-sm" title={text("url") || text("connection")}>{variant === "http" ? text("url") : `${text("connection")} · ${text("event")}`}</span>
      </div>
      {#if summary}<p class="mt-2 text-sm text-muted">{summary}</p>{/if}
      {#if variant === "http" && httpBody}
        <p class="mt-3 mb-1 text-xs font-medium text-muted">{t("approvalCard.body")}</p>
        <CodeBlock code={httpBody} language={args.json !== undefined ? "json" : "text"} maxHeight={200} wrap label={t("approvalCard.requestBody")} />
      {:else if variant === "webhook"}
        <p class="mt-3 mb-1 text-xs font-medium text-muted">{t("approvalCard.payload")}</p>
        <CodeBlock code={JSON.stringify(args.payload ?? {}, null, 2)} language="json" maxHeight={200} label={t("approvalCard.payload")} />
      {/if}
    {:else if variant === "git" || variant === "jira"}
      <p>{t("approvalCard.theRunWantsTo")} <strong class="text-fg">{actionWords(service.action)}</strong> on {service.service === "github" ? t("common.github") : service.service === "gitlab" ? t("approvalCard.gitlab") : t("approvalCard.jira")}.</p>
      <div class="mt-3 flex flex-wrap items-center gap-2 rounded-lg border border-line bg-panel px-3 py-2">
        <Icon name={variant === "git" ? "git-branch" : "list"} size={15} class="text-muted" />
        <span class="mono selectable text-sm">{text("repo") || text("project") || text("key") || ""}{args.number ? ` #${args.number}` : ""}</span>
        {#if text("state")}<Badge tone="muted">{text("state")}</Badge>{/if}
        {#if text("transition")}<Badge tone="accent">{text("transition")}</Badge>{/if}
      </div>
      {#if text("title") || text("summary")}<p class="selectable mt-3 text-md font-medium">{text("title") || text("summary")}</p>{/if}
      {#if body}<blockquote class="selectable mt-2 max-h-52 overflow-auto rounded-lg border border-line bg-panel p-3 text-base leading-relaxed whitespace-pre-wrap text-fg">{body}</blockquote>{/if}
      {#if !text("title") && !body && !text("summary")}<CodeBlock code={json(approval.arguments)} language="json" maxHeight={200} label={t("common.arguments")} class="mt-3" />{/if}
    {:else if variant === "repository"}
      <div class="flex flex-wrap items-center gap-2">
        <p class="min-w-0 flex-1">{summary || approval.tool}</p>
        {#if approval.tool === "git.push"}<Badge tone="warn">{t("approvalCard.leavesThisComputer")}</Badge>{/if}
      </div>
      {#if preview}
        <div class="mt-3">
          <KeyValue
            items={[
              ...(preview.url ? [{ key: t("approvalCard.remote"), value: `${text("remote")} · ${preview.url}`, mono: true }] : []),
              { key: t("approvalCard.branch"), value: preview.new_on_remote ? t("approvalCard.newOnTheRemote", { branch: preview.branch }) : preview.branch, mono: true },
              ...(preview.commit ? [{ key: t("approvalCard.commit"), value: `${preview.commit.slice(0, 7)} ${preview.subject ?? ""}`, mono: true }] : [{ key: t("approvalCard.message"), value: text("message") }]),
              { key: t("approvalCard.files"), value: [...preview.files, ...(preview.new_files ?? []).map((f) => t("approvalCard.newFile", { file: f }))].join(", "), mono: true },
            ]}
          />
        </div>
        {#if preview.diff}<DiffView patch={preview.diff} class="mt-3" />{/if}
        {#if preview.not_shown}<p class="mt-2 text-xs text-muted">{t("approvalCard.filesNotShown", { count: preview.not_shown })}</p>{/if}
      {:else if previewError}
        <Callout tone="err" class="mt-3">{previewError}</Callout>
        <CodeBlock code={json(approval.arguments)} language="json" maxHeight={200} label={t("common.arguments")} class="mt-3" />
      {/if}
      <p class="mt-2 text-xs text-muted">{approval.tool === "git.push" ? t("approvalCard.pushNeverForces") : t("approvalCard.hooksDoNotRun")}</p>
    {:else if variant === "chat"}
      <p>{t("approvalCard.theRunWantsTo2")} <strong class="text-fg">{service.service === "slack" ? t("common.slack") : t("common.discord")}</strong> channel <span class="mono text-fg">{text("channel")}</span>.</p>
      <div class="mt-3 rounded-2xl rounded-tl-sm border border-line bg-panel p-3.5 shadow-1">
        <p class="selectable text-md leading-relaxed whitespace-pre-wrap">{text("text") || text("content")}</p>
      </div>
      <p class="mt-1.5 text-xs text-muted">{t("approvalCard.mentionsOfEveryoneAre")}</p>
    {:else if variant === "db"}
      <p>{t("approvalCard.theRunWantsTo3")} <strong class="mono text-fg">{text("connection")}</strong>{t("approvalCard.queriesCannotChangeData")}</p>
      <CodeBlock code={text("sql")} language="sql" maxHeight={240} wrap label={t("approvalCard.sqlStatement")} class="mt-3" />
      <Callout tone="err" icon="alert" class="mt-3">{t("approvalCard.itRunsInOne")}</Callout>
    {:else if variant === "code"}
      <p>{t("approvalCard.theRunWantsTo4")} <strong class="text-fg">{text("language") === "python" ? t("approvalCard.python") : t("approvalCard.javascript")}</strong> {t("approvalCard.inTheSandboxWith")}</p>
      <CodeBlock code={text("code")} language={text("language") === "python" ? "py" : "js"} maxHeight={260} label={t("approvalCard.program")} class="mt-3" />
    {:else}
      <p>{t("approvalCard.theRunWantsTo5")} <code class="mono text-fg">{approval.tool}</code>{t("approvalCard.nothingHasHappenedYet")}</p>
      {#if summary}<p class="mt-1 text-fg">{summary}</p>{/if}
      <p class="mt-1">{t("approvalCard.whyYouAreAsked", { reason: approval.reason })}</p>
      <CodeBlock code={json(approval.arguments)} language="json" maxHeight={200} label={t("common.arguments")} class="mt-3" />
    {/if}
    {#if variant !== "generic" && variant !== "mcp"}<p class="mt-3 text-xs text-muted">{t("approvalCard.whyYouAreAsked", { reason: approval.reason })}</p>{/if}
    </div>
    {@render answer(t("common.approve"))}
    {#snippet actions()}{@render buttons(t("common.approve"))}{/snippet}
  </Callout>
{/if}
