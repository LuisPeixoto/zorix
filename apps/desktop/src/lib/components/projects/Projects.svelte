<script lang="ts">
  import { projects } from "../../stores/projects.svelte.ts";
  import ProjectPage from "./ProjectPage.svelte";
  import ProjectsPage from "./ProjectsPage.svelte";
</script>

{#if projects.section === "project" && projects.key}
  {#key projects.key}<ProjectPage key={projects.key} />{/key}
{:else}
  <ProjectsPage />
{/if}
