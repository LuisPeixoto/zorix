<script lang="ts">
  import { t } from "../i18n/index.ts";
  import Knowledge from "./Knowledge.svelte";
  import { PageHeader } from "../ui/index.ts";
</script>

<div class="mx-auto flex h-full max-w-4xl flex-col gap-5 overflow-y-auto p-6 [&>*]:shrink-0">
  <PageHeader
    title={t("common.documents")}
    subtitle={t("documents.foldersZorixCanSearch")}
    glow
    class="mb-1"
  />
  <Knowledge />
</div>
