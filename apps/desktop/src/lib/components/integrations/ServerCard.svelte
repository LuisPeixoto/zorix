<script lang="ts">
  import type { ServerRow } from "../../api.ts";
  import { Badge, BrandLogo, Icon, StatusPill, cn } from "../../ui/index.ts";

  let { server, onopen }: { server: ServerRow; onopen: () => void } = $props();

  const pill = $derived(server.tools_to_review > 0 && server.state === "ready" ? "review" : server.state);
  const label = $derived(server.tools_to_review > 0 && server.state === "ready" ? `${server.tools_to_review} to review` : undefined);
</script>

<button
  type="button"
  onclick={onopen}
  class={cn(
    "raised group flex w-full flex-col gap-3 rounded-2xl border border-line bg-panel p-4 text-left transition duration-150 ease-out hover:-translate-y-px hover:border-line-strong hover:shadow-3",
    !server.enabled && "opacity-80",
  )}
>
  <div class="flex items-start gap-3">
    <BrandLogo name={server.logo ?? (server.kind === "http" ? "globe" : "server")} size={40} />
    <div class="min-w-0 flex-1">
      <p class="truncate text-lg font-semibold">{server.name}</p>
      <p class="mono truncate text-xs text-muted" title={server.summary}>{server.summary}</p>
    </div>
    <Icon name="chevron-right" size={16} class="mt-1 text-faint transition duration-150 group-hover:translate-x-0.5 group-hover:text-muted" />
  </div>
  <div class="flex flex-wrap items-center gap-2">
    <StatusPill state={pill} {label} />
    {#if server.enabled && server.state !== "needs_login"}
      <span class="tnum text-xs text-muted">{server.tools_ready} of {server.tools_listed} tools ready</span>
    {/if}
    {#if server.reason}<span class="min-w-0 flex-1 truncate text-xs text-muted" title={server.reason}>{server.reason}</span>{/if}
    <span class="ml-auto flex gap-1.5">
      <Badge tone="muted">{server.kind === "stdio" ? "runs here" : "remote"}</Badge>
    </span>
  </div>
</button>
