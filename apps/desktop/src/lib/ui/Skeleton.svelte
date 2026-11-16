<script lang="ts">
  let { class: klass = "h-4 w-full" }: { class?: string } = $props();
</script>

<div class={`skeleton ${klass}`} aria-hidden="true"></div>
