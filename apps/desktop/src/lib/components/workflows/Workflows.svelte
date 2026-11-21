<script lang="ts">
  import { workflows } from "../../stores/workflows.svelte.ts";
  import WorkflowEditor from "./WorkflowEditor.svelte";
  import WorkflowRunView from "./WorkflowRunView.svelte";
  import WorkflowsPage from "./WorkflowsPage.svelte";
</script>

{#if workflows.section === "editor" && workflows.id}
  {#key workflows.id}<WorkflowEditor id={workflows.id} />{/key}
{:else if workflows.section === "run" && workflows.runId}
  {#key workflows.runId}<WorkflowRunView runId={workflows.runId} />{/key}
{:else}
  <WorkflowsPage />
{/if}
