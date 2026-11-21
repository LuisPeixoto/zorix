<script lang="ts">
  import Knowledge from "./Knowledge.svelte";
  import { PageHeader } from "../ui/index.ts";
</script>

<div class="mx-auto flex h-full max-w-4xl flex-col gap-5 overflow-y-auto p-6 [&>*]:shrink-0">
  <PageHeader
    title="Documents"
    subtitle="Folders ZORIX can search, with citations. Text, code, HTML and PDF files are read on this computer."
    glow
    class="mb-1"
  />
  <Knowledge />
</div>
