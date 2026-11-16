<script lang="ts">
  import type { Snippet } from "svelte";

  import Icon from "./Icon.svelte";
  import { cn } from "./cn.ts";
  import { type UiTone, toneClass } from "./tone.ts";

  let {
    tone = "info",
    title,
    icon,
    actions,
    class: klass = "",
    children,
  }: { tone?: UiTone; title?: string; icon?: string; actions?: Snippet; class?: string; children?: Snippet } = $props();

  const defaults: Record<UiTone, string> = { ok: "check-circle", warn: "alert", err: "x-circle", info: "info", muted: "info", accent: "sparkles" };
</script>

<div role={tone === "err" ? "alert" : "status"} class={cn("flex gap-3 rounded-xl border p-4", toneClass(tone, "callout"), klass)}>
  <Icon name={icon ?? defaults[tone]} size={18} class={cn("mt-0.5", toneClass(tone, "text"))} />
  <div class="min-w-0 flex-1">
    {#if title}<p class={cn("text-md font-semibold", toneClass(tone, "text"))}>{title}</p>{/if}
    {#if children}<div class="text-base text-muted">{@render children()}</div>{/if}
    {#if actions}<div class="mt-3 flex flex-wrap gap-2">{@render actions()}</div>{/if}
  </div>
</div>
