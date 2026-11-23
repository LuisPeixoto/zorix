<script lang="ts">
  import { t } from "../i18n/index.ts";
  import { diffLines, diffWords, summarize } from "./diff.ts";
  import { cn } from "./cn.ts";

  /** Two versions of a text, with what was added and removed. A short sentence is compared word by word, longer text line by line. */
  let {
    before,
    after,
    beforeLabel = t("ui.diffView.approved"),
    afterLabel = t("ui.diffView.now"),
    class: klass = "",
  }: { before: string; after: string; beforeLabel?: string; afterLabel?: string; class?: string } = $props();

  const words = $derived(!before.includes("\n") && !after.includes("\n"));
  const parts = $derived(words ? diffWords(before, after) : diffLines(before, after));
  const counts = $derived(summarize(parts));
</script>

<figure class={cn("overflow-hidden rounded-xl border border-line bg-panel-2", klass)}>
  <figcaption class="flex items-center gap-3 border-b border-line bg-panel px-3 py-1.5 text-xs text-muted">
    <span>{beforeLabel} → {afterLabel}</span>
    <span class="tnum ml-auto text-ok">+{counts.added}</span>
    <span class="tnum text-err">−{counts.removed}</span>
  </figcaption>
  {#if words}
    <p class="selectable p-3 text-base leading-relaxed">
      {#each parts as part, i (i)}{#if part.kind === "same"}<span>{part.text}</span>{:else if part.kind === "add"}<ins class="rounded bg-ok/15 px-0.5 text-ok no-underline"><span class="sr-only">added: </span>{part.text}</ins>{:else}<del class="rounded bg-err/15 px-0.5 text-err"><span class="sr-only">removed: </span>{part.text}</del>{/if}{/each}
    </p>
  {:else}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex (a scrollable region must be reachable by keyboard) -->
    <div class="mono selectable overflow-auto py-1 text-sm" role="region" aria-label={t("ui.diffView.changes")} tabindex="0">
      {#each parts as part, i (i)}
        {#each part.text.split("\n") as line, j (j)}
          <div
            class={cn(
              "flex gap-2 px-3 whitespace-pre",
              part.kind === "add" && "bg-ok/10 text-ok",
              part.kind === "del" && "bg-err/10 text-err",
            )}
          >
            <span class="w-3 shrink-0 select-none" aria-hidden="true">{part.kind === "add" ? "+" : part.kind === "del" ? "−" : " "}</span>
            <span>{line}</span>
          </div>
        {/each}
      {/each}
    </div>
  {/if}
</figure>
