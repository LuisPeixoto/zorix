<script lang="ts">
  import { t } from "../../i18n/index.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { Button, Card, CodeBlock, ToggleRow } from "../../ui/index.ts";
  import { serveCommand } from "./serveCommand.ts";

  // Nothing here is saved or run: the switches only build the command the person gives to the other tool.
  let runs = $state(false);
  let approvals = $state(false);
  let readRuns = $state(false);

  const command = $derived(serveCommand({ runs, approvals, readRuns }));

  async function copy() {
    try {
      await navigator.clipboard.writeText(command);
      toast.ok(t("settings.otherTools.copied"));
    } catch {
      toast.err(t("settings.otherTools.notCopied"));
    }
  }
</script>

<Card title={t("settings.otherTools.title")} icon="server">
  <p class="mb-3 text-base text-muted">{t("settings.otherTools.intro")}</p>
  <CodeBlock code={command} wrap copy={false} label={t("settings.otherTools.command")} />
  <div class="mt-1 -mx-4">
    <ToggleRow label={t("settings.otherTools.runs")} description={t("settings.otherTools.runsNote")} bind:checked={runs} />
    <ToggleRow label={t("settings.otherTools.approvals")} description={t("settings.otherTools.approvalsNote")} disabled={!runs} bind:checked={approvals} />
    <ToggleRow label={t("settings.otherTools.readRuns")} description={t("settings.otherTools.readRunsNote")} disabled={!runs} bind:checked={readRuns} />
  </div>
  <div class="mt-3 flex justify-end">
    <Button variant="primary" icon="copy" onclick={() => void copy()}>{t("settings.otherTools.copy")}</Button>
  </div>
</Card>
