<script lang="ts">
  import { type Language, highlight, markJsonKeys } from "../ui/highlight.ts";
  import IconButton from "./IconButton.svelte";
  import { cn } from "./cn.ts";

  let {
    code,
    language = "text",
    maxHeight = 280,
    wrap = false,
    copy = true,
    label = "Code",
    class: klass = "",
  }: { code: string; language?: Language; maxHeight?: number; wrap?: boolean; copy?: boolean; label?: string; class?: string } = $props();

  const tokens = $derived(language === "json" ? markJsonKeys(highlight(code, language)) : highlight(code, language));
  let copied = $state(false);

  async function copyCode() {
    try {
      await navigator.clipboard.writeText(code);
      copied = true;
      setTimeout(() => (copied = false), 1400);
    } catch {
      /* The clipboard may be blocked; the text is still selectable. */
    }
  }
</script>

<div class={cn("group relative overflow-hidden rounded-xl border border-line bg-panel-2", klass)}>
  <!-- svelte-ignore a11y_no_noninteractive_tabindex (a scrollable region must be reachable by keyboard) -->
  <div class="mono selectable overflow-auto p-3 text-sm leading-relaxed" style="max-height:{maxHeight}px" role="region" aria-label={label} tabindex="0">
    <pre class={cn("m-0 font-[inherit]", wrap ? "whitespace-pre-wrap break-words" : "whitespace-pre")}>{#each tokens as token, i (i)}<span class={token.kind === "plain" ? "" : `tok-${token.kind}`}>{token.text}</span>{/each}</pre>
  </div>
  {#if copy}
    <div class="absolute top-1.5 right-1.5 opacity-0 transition duration-150 group-focus-within:opacity-100 group-hover:opacity-100">
      <IconButton icon={copied ? "check" : "copy"} label={copied ? "Copied" : "Copy"} variant="secondary" size="sm" onclick={copyCode} />
    </div>
  {/if}
</div>
