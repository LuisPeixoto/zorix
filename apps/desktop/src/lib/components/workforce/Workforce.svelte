<script lang="ts">
  import { workforce } from "../../stores/workforce.svelte.ts";
  import WorkerPage from "./WorkerPage.svelte";
  import WorkforcePage from "./WorkforcePage.svelte";
</script>

{#if workforce.section === "worker" && workforce.selected}
  <WorkerPage />
{:else}
  <WorkforcePage />
{/if}
