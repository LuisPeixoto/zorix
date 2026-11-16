<script lang="ts">
  import { cn } from "./cn.ts";
  import { type UiTone, toneClass } from "./tone.ts";

  let { tone = "muted", live = false, class: klass = "" }: { tone?: UiTone; live?: boolean; class?: string } = $props();
</script>

<span class={cn("inline-block size-2 shrink-0 rounded-full", toneClass(tone, "dot"), toneClass(tone, "text"), live && "live-dot", klass)} aria-hidden="true"></span>
