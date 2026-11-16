<script lang="ts">
  import Icon from "./Icon.svelte";
  let { size = 16, class: klass = "" }: { size?: number; class?: string } = $props();
</script>

<Icon name="loader" {size} class={`animate-spin ${klass}`} />
