<script lang="ts">
  import { fmt, t } from "../../i18n/index.ts";
  import { type AttachedFile, MAX_ATTACHED } from "../../attach.ts";
  import { Button, Callout, Chip, Icon } from "../../ui/index.ts";

  /** The files a run is given, as chips, with the button that adds more and what a person should know about them. */
  let {
    files,
    onremove,
    onattach,
    disabled = false,
    compact = false,
  }: { files: AttachedFile[]; onremove: (path: string) => void; onattach: () => void; disabled?: boolean; compact?: boolean } = $props();

  const outside = $derived(files.filter((f) => f.outside));
</script>

<div class="space-y-2">
  {#if !compact || files.length > 0}
    <p class="text-sm font-medium">{t("attach.attached")} <span class="font-normal text-faint">· {t("attach.countOfMax", { count: files.length, max: MAX_ATTACHED })}</span></p>
  {/if}
  <ul class="flex flex-wrap items-center gap-2" aria-label={t("attach.attached")}>
    {#each files as file (file.path)}
      <li>
        <Chip selected onremove={() => onremove(file.path)}>
          <span class="inline-flex items-center gap-1.5"><Icon name="file-text" size={13} /><span class="mono text-xs">{file.name}</span><span class="text-2xs font-normal text-faint">{fmt.bytes(file.bytes)}</span></span>
        </Chip>
      </li>
    {/each}
    <li><Button variant="ghost" size="sm" icon="paperclip" {disabled} onclick={onattach}>{t("attach.attach")}</Button></li>
  </ul>
  {#if !compact}<p class="text-xs text-muted">{t("attach.theRunReadsThem")}</p>{/if}
  {#each outside as file (file.path)}
    <Callout tone="warn" icon="info" title={t("attach.isOutsideTheWorkspace", { name: file.name })}>{t("attach.youChoseItSo")}</Callout>
  {/each}
</div>
