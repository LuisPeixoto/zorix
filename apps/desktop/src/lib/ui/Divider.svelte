<script lang="ts">
  let { vertical = false, class: klass = "" }: { vertical?: boolean; class?: string } = $props();
</script>

<div role="separator" aria-orientation={vertical ? "vertical" : "horizontal"} class={`bg-line ${vertical ? "h-5 w-px" : "h-px w-full"} ${klass}`}></div>
