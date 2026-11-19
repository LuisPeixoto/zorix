<script lang="ts">
  import type { Snippet } from "svelte";

  import { cn } from "./cn.ts";

  /** One message of a conversation. The person's sit on the right, the agent's on the left beside its face. */
  let {
    role,
    name,
    time,
    streaming = false,
    avatar,
    children,
  }: { role: "user" | "assistant"; name?: string; time?: string; streaming?: boolean; avatar?: Snippet; children: Snippet } = $props();
</script>

<article class={cn("flex gap-3", role === "user" ? "flex-row-reverse" : "flex-row")} aria-label={role === "user" ? "You" : (name ?? "Agent")}>
  {#if role === "assistant"}<div class="mt-0.5 shrink-0">{@render avatar?.()}</div>{/if}
  <div class={cn("flex min-w-0 max-w-[min(46rem,88%)] flex-col gap-1", role === "user" ? "items-end" : "items-start")}>
    {#if name || time}
      <p class="flex items-baseline gap-2 text-xs text-muted">
        {#if name}<span class="font-medium text-fg/80">{name}</span>{/if}
        {#if time}<time class="tnum">{time}</time>{/if}
      </p>
    {/if}
    <div
      class={cn(
        "min-w-0 rounded-2xl px-4 py-2.5 text-base",
        role === "user"
          ? "rounded-tr-md bg-[image:var(--accent-grad)] bg-accent text-accent-fg shadow-[inset_0_1px_0_rgb(255_255_255/0.18),0_6px_16px_-10px_var(--accent)]"
          : "raised rounded-tl-md border border-line bg-panel",
        streaming && "stream-caret",
      )}
    >
      {@render children()}
    </div>
  </div>
</article>

<style>
  /* A soft caret after the text while the answer is still being written. */
  :global(.stream-caret) > :global(*:last-child)::after,
  :global(.stream-caret):empty::after {
    content: "";
    display: inline-block;
    width: 0.5ch;
    height: 1.05em;
    margin-left: 2px;
    vertical-align: text-bottom;
    border-radius: 2px;
    background: var(--accent);
    animation: caret 1s ease-in-out infinite;
  }
  @keyframes caret {
    50% {
      opacity: 0.15;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    :global(.stream-caret) > :global(*:last-child)::after {
      animation: none;
    }
  }
</style>
