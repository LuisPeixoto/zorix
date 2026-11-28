<script lang="ts">
  import { t as tr } from "../../i18n/index.ts";
  import type { McpToolRow } from "../../api.ts";
  import { integrations } from "../../stores/integrations.svelte.ts";
  import SandboxSection from "./SandboxSection.svelte";
  import {
    Badge, BrandLogo, Button, Callout, CodeBlock, DiffView, Drawer, KeyValue, Modal, StatusPill, Switch, Tabs, ToggleRow, cn,
  } from "../../ui/index.ts";

  let open = $state(true);
  const detail = $derived(integrations.detail);
  let tab = $state("tools");
  let confirmRemove = $state(false);
  let expanded = $state<string | null>(null);

  $effect(() => {
    // Closing the drawer forgets the server on screen.
    if (!open) integrations.close();
  });

  const signingIn = $derived(!!detail && integrations.signingIn.includes(detail.name));
  const toReview = $derived(detail?.tools.filter((t) => t.status === "new" || t.status === "changed") ?? []);
  const tabs = $derived([
    { value: "tools", label: tr("integrations.serverDrawer.tools"), count: detail?.tools.length },
    { value: "details", label: tr("integrations.serverDrawer.details") },
    { value: "logs", label: tr("integrations.serverDrawer.logs") },
  ]);

  const statusTone = (t: McpToolRow) => (t.status === "pinned" ? "ok" : t.status === "changed" ? "warn" : t.status === "new" ? "accent" : "muted");
  const statusText: Record<McpToolRow["status"], string> = { pinned: "approved", new: "new", changed: "changed", disabled: "off" };
  const risk = (t: McpToolRow) => (t.destructive_hint ? tr("integrations.serverDrawer.canChangeThings") : t.read_only_hint ? tr("integrations.serverDrawer.readsOnly") : "unclear");
</script>

<Drawer bind:open title={detail?.name ?? integrations.openName ?? ""} subtitle={detail ? `${detail.server_name ?? tr("integrations.serverDrawer.mcpServer")}${detail.server_version ? " " + detail.server_version : ""}` : ""} width="lg">
  {#snippet leading()}
    <BrandLogo name={detail?.logo ?? (detail?.kind === "http" ? "globe" : "server")} size={40} />
  {/snippet}
  {#snippet actions()}
    {#if detail}
      <StatusPill state={detail.tools_to_review > 0 && detail.state === "ready" ? "review" : detail.state} />
      <Switch
        checked={detail.enabled}
        aria-label={tr("integrations.serverDrawer.turnOn", { name: detail.name })}
        onchange={(e) => void integrations.setEnabled(detail.name, e.currentTarget.checked)}
      />
    {/if}
  {/snippet}

  {#if !detail}
    <div class="space-y-3 p-5" aria-busy="true" aria-label={tr("common.loading")}>
      <div class="skeleton h-16"></div><div class="skeleton h-24"></div><div class="skeleton h-24"></div>
    </div>
  {:else}
    <div class="space-y-3 px-5 pt-4">
      {#if detail.state === "needs_login"}
        <Callout tone="warn" title={tr("integrations.serverDrawer.signInToUse", { name: detail.name })} icon="log-in">
          {detail.reason ?? tr("integrations.serverDrawer.thisServerAsksFor")}
          {#snippet actions()}
            {#if signingIn}
              <Button size="sm" onclick={() => integrations.cancelLogin(detail.name)}>{tr("common.cancel")}</Button>
              <span class="self-center text-sm text-muted">{tr("integrations.serverDrawer.waitingForYourBrowser")}</span>
            {:else}
              <Button size="sm" variant="primary" icon="log-in" onclick={() => integrations.login(detail.name)}>{tr("integrations.serverDrawer.signIn")}</Button>
            {/if}
          {/snippet}
        </Callout>
      {:else if detail.state === "failed"}
        <Callout tone="err" title={tr("integrations.serverDrawer.couldNotStart", { name: detail.name })}>
          <span class="selectable">{detail.reason}</span>
          {#snippet actions()}<Button size="sm" icon="refresh" onclick={() => integrations.restart(detail.name)}>{tr("integrations.serverDrawer.tryAgain")}</Button>{/snippet}
        </Callout>
      {/if}
      {#if toReview.length > 0 && detail.enabled}
        <Callout tone="accent" icon="shield-check" title={tr("integrations.serverDrawer.toolsToReview", { count: toReview.length })}>
          {tr("integrations.serverDrawer.toolsAreNotOffered")} {#if toReview.some((t) => t.status === "changed")}<strong class="text-fg">{tr("integrations.serverDrawer.someChangedSinceYou")}</strong>{tr("integrations.serverDrawer.readWhatIsDifferent")}{/if}
          {#snippet actions()}<Button size="sm" variant="primary" icon="check" onclick={() => integrations.approveAll(detail.name, toReview.map((t) => [t.name, t.hash]))}>{tr("integrations.serverDrawer.approveAll", { length: toReview.length })}</Button>{/snippet}
        </Callout>
      {/if}
    </div>

    <div class="px-5 pt-4"><Tabs {tabs} bind:value={tab} label={tr("integrations.serverDrawer.serverSections")} /></div>

    {#if tab === "tools"}
      {#if detail.tools.length === 0}
        <p class="px-5 py-8 text-center text-base text-muted">
          {detail.enabled ? (detail.state === "ready" ? tr("integrations.serverDrawer.thisServerListsNo") : tr("integrations.serverDrawer.itsToolsAppearHere")) : tr("integrations.serverDrawer.turnTheServerOn")}
        </p>
      {:else}
        <ul class="mt-3 divide-y divide-line-soft border-y border-line-soft" aria-label={tr("integrations.serverDrawer.tools")}>
          {#each detail.tools as tool (tool.name)}
            <li class={cn(tool.status === "changed" && "bg-warn/6", tool.status === "new" && "bg-accent-soft/50")}>
              <div class="flex items-start gap-3 px-5 py-3">
                <div class="min-w-0 flex-1">
                  <button type="button" class="flex w-full items-center gap-2 text-left" aria-expanded={expanded === tool.name} onclick={() => (expanded = expanded === tool.name ? null : tool.name)}>
                    <span class="mono truncate text-md font-medium">{tool.name}</span>
                    <Badge tone={statusTone(tool)}>{statusText[tool.status]}</Badge>
                    <Badge tone="muted" class="hidden sm:inline-flex">{risk(tool)}</Badge>
                  </button>
                  <p class={cn("selectable mt-1 text-sm text-muted", expanded === tool.name ? "" : "line-clamp-2")}>{tool.description || tr("integrations.serverDrawer.noDescription")}</p>
                </div>
                {#if tool.status === "pinned" || tool.status === "disabled"}
                  <Switch
                    checked={tool.status === "pinned"}
                    aria-label={tr("integrations.serverDrawer.offerToRuns", { name: tool.name })}
                    onchange={(e) => void integrations.review(detail.name, tool.name, e.currentTarget.checked ? "approve" : "disable", tool.hash)}
                  />
                {:else}
                  <div class="flex shrink-0 gap-1.5">
                    <Button size="sm" variant="primary" aria-label={tool.status === "changed" ? tr("integrations.serverDrawer.approveTheChangeTo", { name: tool.name }) : tr("integrations.serverDrawer.approveTool", { name: tool.name })} onclick={() => integrations.review(detail.name, tool.name, "approve", tool.hash)}>{tool.status === "changed" ? tr("integrations.serverDrawer.approveChange") : tr("common.approve")}</Button>
                    <Button size="sm" aria-label={tr("integrations.serverDrawer.turnOff", { name: tool.name })} onclick={() => integrations.review(detail.name, tool.name, "disable")}>{tr("integrations.serverDrawer.off")}</Button>
                  </div>
                {/if}
              </div>
              {#if tool.status === "changed"}
                <div class="space-y-2 px-5 pb-3">
                  {#if tool.approved_description !== null && tool.approved_description !== tool.description}
                    <DiffView before={tool.approved_description} after={tool.description} />
                  {/if}
                  <p class="text-xs text-muted">{tr("integrations.serverDrawer.changed")} {tool.changed_fields.join(", ") || tr("integrations.serverDrawer.itsDefinition")}.</p>
                </div>
              {/if}
              {#if expanded === tool.name}
                <div class="space-y-2 px-5 pb-3">
                  {#if tool.inputs.length}<p class="text-xs text-muted">{tr("integrations.serverDrawer.takes")} <span class="mono">{tool.inputs.join(", ")}</span></p>{/if}
                  <CodeBlock code={JSON.stringify(tool.schema, null, 2)} language="json" maxHeight={200} label={tr("integrations.serverDrawer.argumentsOf", { name: tool.name })} />
                </div>
              {/if}
              {#if tool.status === "pinned" && tool.can_skip_asking}
                <ToggleRow
                  class="pt-0 pl-8"
                  checked={tool.trust === "read_only_no_ask"}
                  label={tr("integrations.serverDrawer.donTAskBefore")}
                  description={tr("integrations.serverDrawer.theServerSaysIt")}
                  onchange={(on) => void integrations.trust(detail.name, tool.name, on)}
                />
              {/if}
            </li>
          {/each}
        </ul>
        <p class="px-5 py-3 text-xs text-muted">
          {tr("integrations.serverDrawer.everyOtherCallAsks")}
        </p>
      {/if}
    {:else if tab === "details"}
      <div class="space-y-5 p-5">
        <KeyValue
          items={[
            { key: "Kind", value: detail.kind === "stdio" ? "A program on this computer" : "A remote server" },
            { key: "Command", value: detail.command ? `${detail.command} ${detail.args.join(" ")}`.trim() : null, mono: true },
            { key: "Address", value: detail.url, mono: true },
            { key: "Folder", value: detail.cwd, mono: true },
            { key: "Protocol", value: detail.era ? (detail.era === "modern" ? "2026-07-28 (stateless)" : "Earlier (with a handshake)") : "Not seen yet" },
            { key: "Sign-in", value: detail.auth === "oauth" ? (detail.signed_in ? "Signed in with the browser" : "Not signed in") : detail.auth === "bearer" ? (detail.signed_in ? "A token is kept" : "No token") : detail.auth === "none" ? "None" : null },
          ]}
        />
        {#if detail.env.length}
          <div>
            <p class="mb-1.5 text-sm font-medium">{tr("integrations.serverDrawer.variablesItReceives")}</p>
            <ul class="space-y-1">
              {#each detail.env as [name, secret, value] (name)}
                <li class="flex items-center gap-2 text-sm"><span class="mono">{name}</span>{#if secret}<Badge tone="muted">{tr("integrations.serverDrawer.secretInTheKeychain")}</Badge>{:else}<span class="mono text-muted">= {value}</span>{/if}</li>
              {/each}
            </ul>
          </div>
        {/if}
        {#if detail.kind === "stdio"}
          <SandboxSection {detail} />
        {/if}
        <div class="flex flex-wrap gap-2">
          <Button icon="refresh" disabled={detail.state !== "ready"} onclick={() => integrations.refreshTools(detail.name)}>{tr("integrations.serverDrawer.refreshTools")}</Button>
          <Button icon="play" disabled={!detail.enabled} onclick={() => integrations.restart(detail.name)}>{tr("integrations.serverDrawer.restart")}</Button>
          {#if detail.auth === "oauth" && detail.signed_in}<Button icon="log-in" onclick={() => integrations.logout(detail.name)}>{tr("integrations.serverDrawer.signOut")}</Button>{/if}
          {#if detail.auth === "oauth" && !detail.signed_in && detail.state !== "needs_login"}<Button icon="log-in" onclick={() => integrations.login(detail.name)}>{tr("integrations.serverDrawer.signIn")}</Button>{/if}
          <Button variant="danger" icon="trash" onclick={() => (confirmRemove = true)}>{tr("common.remove")}</Button>
        </div>
      </div>
    {:else}
      <div class="p-5">
        {#if detail.log.length}
          <CodeBlock code={detail.log.join("\n")} language="text" maxHeight={420} wrap label={tr("integrations.serverDrawer.whatWroteOnIts", { name: detail.name })} />
          <p class="mt-2 text-xs text-muted">{tr("integrations.serverDrawer.secretsAreRemovedFrom")}</p>
        {:else}
          <p class="py-8 text-center text-base text-muted">{tr("integrations.serverDrawer.hasNotWrittenAnything", { name: detail.name })}</p>
        {/if}
      </div>
    {/if}
  {/if}
</Drawer>

<Modal bind:open={confirmRemove} title={tr("integrations.serverDrawer.remove2", { name: detail?.name })} icon="trash" size="sm">
  <p class="text-base text-muted">{tr("integrations.serverDrawer.theServerWhatZorix")}</p>
  {#snippet footer()}
    <Button onclick={() => (confirmRemove = false)}>{tr("common.cancel")}</Button>
    <Button variant="danger" icon="trash" onclick={() => { confirmRemove = false; if (detail) void integrations.remove(detail.name); }}>{tr("common.remove")}</Button>
  {/snippet}
</Modal>
