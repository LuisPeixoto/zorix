<script lang="ts">
  import CodeBlock from "./CodeBlock.svelte";
  import { type Block, type Inline, parseMarkdown } from "./markdown.ts";
  import type { Language } from "./highlight.ts";
  import { cn } from "./cn.ts";

  /** Text written by a model, shown safely: the tree comes from `markdown.ts`, so no markup from the text ever reaches the page. */
  let {
    text,
    onlink,
    class: klass = "",
  }: { text: string; onlink?: (href: string) => void; class?: string } = $props();

  const blocks = $derived(parseMarkdown(text));
  const LANGS: Record<string, Language> = { sql: "sql", json: "json", js: "js", javascript: "js", ts: "js", typescript: "js", py: "py", python: "py" };

  function follow(event: MouseEvent, href: string) {
    if (!onlink) return;
    event.preventDefault();
    onlink(href);
  }
  const align = (a: "left" | "right" | "center" | null) => (a === "right" ? "text-right" : a === "center" ? "text-center" : "text-left");
</script>

{#snippet inline(nodes: Inline[])}
  {#each nodes as node, k (k)}
    {#if node.t === "text"}{node.text}
    {:else if node.t === "code"}<code class="mono rounded-md bg-hover px-1 py-0.5 text-sm">{node.text}</code>
    {:else if node.t === "strong"}<strong class="font-semibold">{@render inline(node.children)}</strong>
    {:else if node.t === "em"}<em>{@render inline(node.children)}</em>
    {:else if node.t === "del"}<del class="text-muted">{@render inline(node.children)}</del>
    {:else if node.t === "link"}<a href={node.href} target="_blank" rel="noopener noreferrer nofollow" title={node.href} class="text-accent underline decoration-accent/40 underline-offset-2 hover:decoration-accent" onclick={(e) => follow(e, node.href)}>{@render inline(node.children)}</a>
    {:else}<br />
    {/if}
  {/each}
{/snippet}

{#snippet flow(items: Block[], tight: boolean)}
  {#each items as block, k (k)}
    {#if block.t === "heading"}
      <p class={cn("mt-4 mb-1.5 font-semibold first:mt-0", block.level === 1 ? "text-xl" : block.level === 2 ? "text-lg" : "text-md")} role="heading" aria-level={Math.min(block.level + 2, 6)}>{@render inline(block.children)}</p>
    {:else if block.t === "p"}
      <p class={cn(tight ? "" : "mb-2.5 last:mb-0")}>{@render inline(block.children)}</p>
    {:else if block.t === "code"}
      <CodeBlock class="my-2.5" code={block.text} language={LANGS[block.lang.toLowerCase()] ?? "text"} label={block.lang || "Code"} maxHeight={360} wrap />
    {:else if block.t === "quote"}
      <blockquote class="my-2.5 border-l-2 border-accent/40 pl-3 text-muted">{@render flow(block.children, false)}</blockquote>
    {:else if block.t === "list"}
      {#if block.ordered}
        <ol start={block.start} class="my-2 list-decimal space-y-1 pl-5 marker:text-faint">
          {#each block.items as item, j (j)}<li>{@render flow(item, item.length === 1)}</li>{/each}
        </ol>
      {:else}
        <ul class="my-2 list-disc space-y-1 pl-5 marker:text-faint">
          {#each block.items as item, j (j)}<li>{@render flow(item, item.length === 1)}</li>{/each}
        </ul>
      {/if}
    {:else if block.t === "table"}
      <div class="my-2.5 overflow-x-auto rounded-xl border border-line">
        <table class="w-full border-collapse text-sm">
          <thead class="bg-panel-2">
            <tr>{#each block.head as cell, c (c)}<th scope="col" class={cn("border-b border-line px-3 py-1.5 font-semibold", align(block.align[c]))}>{@render inline(cell)}</th>{/each}</tr>
          </thead>
          <tbody>
            {#each block.rows as row, r (r)}
              <tr class="border-t border-line-soft">{#each row as cell, c (c)}<td class={cn("px-3 py-1.5", align(block.align[c]))}>{@render inline(cell)}</td>{/each}</tr>
            {/each}
          </tbody>
        </table>
      </div>
    {:else}
      <hr class="my-4 border-line" />
    {/if}
  {/each}
{/snippet}

<div class={cn("selectable text-base leading-relaxed break-words", klass)}>{@render flow(blocks, false)}</div>
