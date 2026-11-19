<script lang="ts">
  import Icon from "./Icon.svelte";
  import { cn } from "./cn.ts";

  /** Where a person is in a short flow. Steps before the current one are done. */
  let { steps, current, class: klass = "" }: { steps: { label: string; hint?: string }[]; current: number; class?: string } = $props();
</script>

<ol class={cn("flex items-start gap-2", klass)} aria-label="Progress">
  {#each steps as step, i (step.label)}
    {@const done = i < current}
    {@const here = i === current}
    <li class="flex min-w-0 flex-1 flex-col gap-1.5" aria-current={here ? "step" : undefined}>
      <div class="flex items-center gap-2">
        <span
          class={cn(
            "grid size-6 shrink-0 place-items-center rounded-full text-xs font-semibold transition duration-200",
            done && "bg-ok text-bg",
            here && "bg-accent text-accent-fg ring-4 ring-accent-soft",
            !done && !here && "border border-line-strong bg-panel-2 text-muted",
          )}
        >
          {#if done}<Icon name="check" size={13} stroke={2.4} />{:else}<span class="tnum">{i + 1}</span>{/if}
        </span>
        {#if i < steps.length - 1}<span class={cn("h-px flex-1", done ? "bg-ok/50" : "bg-line")}></span>{/if}
      </div>
      <p class={cn("truncate text-sm font-medium", here ? "text-fg" : "text-muted")}>{step.label}<span class="sr-only">{done ? " (done)" : here ? " (current)" : ""}</span></p>
      {#if step.hint}<p class="truncate text-xs text-muted">{step.hint}</p>{/if}
    </li>
  {/each}
</ol>
