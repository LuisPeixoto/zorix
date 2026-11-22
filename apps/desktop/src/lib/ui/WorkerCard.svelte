<script lang="ts">
  import Avatar from "./Avatar.svelte";
  import { cn } from "./cn.ts";

  /**
   * A worker in the organisation: its face, name, role, status and three figures. A person's card is dashed with a round avatar, so
   * a person is never taken for an agent (design system: WorkerCard).
   */
  let {
    name,
    role,
    emoji = "🤖",
    color = "accent",
    status = null,
    human = false,
    initials,
    figures,
    selected = false,
    onclick,
    label,
    class: klass = "",
  }: {
    name: string;
    role: string;
    emoji?: string;
    color?: string;
    status?: "ok" | "warn" | "err" | null;
    human?: boolean;
    initials?: string;
    figures: [value: string, label: string][];
    selected?: boolean;
    onclick?: () => void;
    label?: string;
    class?: string;
  } = $props();
</script>

<button
  type="button"
  {onclick}
  aria-pressed={selected}
  aria-label={label}
  class={cn(
    "flex w-49 flex-col gap-2.5 rounded-xl border bg-panel p-3 text-left shadow-2 transition duration-150 ease-out outline-none hover:border-line-strong focus-visible:ring-2 focus-visible:ring-accent/60",
    human ? "border-dashed border-line" : "border-line",
    selected && "border-accent ring-2 ring-accent/30",
    klass,
  )}
>
  <span class="flex min-w-0 items-center gap-2.5">
    <Avatar {emoji} {color} {human} {initials} {status} class="size-8 rounded-xl {human ? 'rounded-full!' : ''}" />
    <span class="min-w-0">
      <span class="block truncate text-base font-semibold">{name}</span>
      <span class="block truncate text-xs text-muted">{role}</span>
    </span>
  </span>
  <span class="flex justify-between border-t border-line-soft pt-2 text-2xs text-muted">
    {#each figures as [value, caption] (caption)}
      <span><b class="block text-sm font-semibold text-fg tabular-nums">{value}</b>{caption}</span>
    {/each}
  </span>
</button>
