<script lang="ts">
  import { fly } from "svelte/transition";

  import { toast } from "../stores/toast.svelte.ts";
  import Icon from "./Icon.svelte";
  import { cn } from "./cn.ts";

  const icon = { ok: "check-circle", err: "x-circle", info: "info" } as const;
  const tone = { ok: "text-ok", err: "text-err", info: "text-accent" } as const;
</script>

<div class="pointer-events-none fixed right-4 bottom-4 z-[60] flex w-80 max-w-[calc(100vw-2rem)] flex-col gap-2" aria-live="polite">
  {#each toast.items as item (item.id)}
    <div
      transition:fly={{ y: 12, duration: 180 }}
      role={item.kind === "err" ? "alert" : "status"}
      class="pointer-events-auto flex items-start gap-2.5 rounded-xl border border-line bg-elevated px-3.5 py-2.5 text-base shadow-pop"
    >
      <Icon name={icon[item.kind]} size={17} class={cn("mt-px", tone[item.kind])} />
      <p class="selectable min-w-0 flex-1 break-words">{item.text}</p>
      <button type="button" onclick={() => toast.dismiss(item.id)} aria-label="Dismiss" class="grid size-5 place-items-center rounded text-faint hover:text-fg"><Icon name="x" size={13} /></button>
    </div>
  {/each}
</div>
