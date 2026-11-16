<script lang="ts">
  import type { Snippet } from "svelte";

  type Ids = { id: string; describedby: string | undefined; invalid: boolean };
  let { label, hint, error, required = false, children }: { label: string; hint?: string; error?: string | null; required?: boolean; children: Snippet<[Ids]> } = $props();

  const uid = $props.id();
  const id = `${uid}-control`;
  const describedby = $derived(error ? `${uid}-error` : hint ? `${uid}-hint` : undefined);
</script>

<div class="flex flex-col gap-1.5">
  <label for={id} class="text-sm font-medium text-fg">
    {label}{#if required}<span class="ml-0.5 text-err" aria-hidden="true">*</span>{/if}
  </label>
  {@render children({ id, describedby, invalid: !!error })}
  {#if error}
    <p id="{uid}-error" class="text-xs text-err" role="alert">{error}</p>
  {:else if hint}
    <p id="{uid}-hint" class="text-xs text-muted">{hint}</p>
  {/if}
</div>
