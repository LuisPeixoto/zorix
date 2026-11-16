<script lang="ts">
  import type { HTMLTextareaAttributes } from "svelte/elements";

  import { cn } from "./cn.ts";

  type Props = HTMLTextareaAttributes & { value?: string; invalid?: boolean; mono?: boolean };
  let { value = $bindable(""), invalid = false, mono = false, class: klass = "", ...rest }: Props = $props();
</script>

<textarea
  bind:value
  aria-invalid={invalid || undefined}
  class={cn(
    "w-full resize-none rounded-lg border bg-panel-2 p-3 text-base leading-relaxed text-fg outline-none transition duration-150 placeholder:text-faint hover:border-line-strong focus:bg-panel focus:ring-3",
    mono && "font-mono text-sm",
    invalid ? "border-err focus:border-err focus:ring-err/20" : "border-line focus:border-accent focus:ring-accent-soft",
    klass,
  )}
  {...rest}
></textarea>
