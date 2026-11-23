<script lang="ts">
  import { t } from "../i18n/index.ts";
  let { class: klass = t("ui.skeleton.h4WFull") }: { class?: string } = $props();
</script>

<div class={`skeleton ${klass}`} aria-hidden="true"></div>
