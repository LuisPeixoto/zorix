<script lang="ts">
  import { onDestroy, tick } from "svelte";

  import { type AttachCandidate, attachmentCandidates } from "../../api.ts";
  import { type Mention, insertMention, mentionAt } from "../../attach.ts";
  import { t } from "../../i18n/index.ts";
  import { Icon, Kbd } from "../../ui/index.ts";

  /**
   * The menu that opens when a person types `@` in a text box: indexed documents and files of the workspace folder. It follows the box it is given (typing,
   * the arrow keys, Enter or Tab to choose, Escape to close) and puts the chosen name into the text where the `@` was.
   */
  let {
    field,
    value = $bindable(""),
    workspace,
    onchoose,
    placement = "below",
  }: { field: HTMLTextAreaElement | undefined; value: string; workspace?: string; onchoose: (path: string) => void; placement?: "below" | "above" } = $props();

  const uid = $props.id();
  let mention = $state<Mention | null>(null);
  let documents = $state<AttachCandidate[]>([]);
  let files = $state<AttachCandidate[]>([]);
  let active = $state(0);
  let timer: ReturnType<typeof setTimeout> | null = null;
  let asked = 0;

  const options = $derived([...documents.map((c) => ({ ...c, group: "documents" as const })), ...files.map((c) => ({ ...c, group: "files" as const }))]);
  const open = $derived(mention !== null && options.length > 0);

  function close() {
    mention = null;
    documents = [];
    files = [];
    active = 0;
  }

  function look() {
    if (!field) return;
    const found = mentionAt(field.value, field.selectionStart ?? 0);
    if (!found) return close();
    mention = found;
    if (timer) clearTimeout(timer);
    const turn = ++asked;
    timer = setTimeout(() => {
      void attachmentCandidates(found.query, workspace)
        .then((list) => {
          // An answer to a question that has since changed is not shown.
          if (turn !== asked || mention === null) return;
          documents = list.documents;
          files = list.files;
          active = 0;
        })
        .catch(() => close());
    }, 120);
  }

  async function choose(candidate: AttachCandidate) {
    if (!mention || !field) return;
    const next = insertMention(field.value, mention, candidate.name);
    value = next.text;
    onchoose(candidate.path);
    close();
    await tick();
    field.focus();
    field.setSelectionRange(next.caret, next.caret);
  }

  // The keys are taken before the box's own handlers see them, so that Enter chooses a file instead of sending the message.
  function keys(event: KeyboardEvent) {
    if (!open || event.isComposing) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      active = (active + (event.key === "ArrowDown" ? 1 : options.length - 1)) % options.length;
    } else if (event.key === "Enter" || event.key === "Tab") {
      void choose(options[active]);
    } else if (event.key === "Escape") {
      close();
    } else {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
  }

  $effect(() => {
    const box = field;
    if (!box) return;
    const watch = () => look();
    box.addEventListener("keydown", keys, true);
    box.addEventListener("input", watch);
    box.addEventListener("click", watch);
    box.addEventListener("blur", close);
    return () => {
      box.removeEventListener("keydown", keys, true);
      box.removeEventListener("input", watch);
      box.removeEventListener("click", watch);
      box.removeEventListener("blur", close);
    };
  });
  onDestroy(() => timer && clearTimeout(timer));
</script>

{#if open}
  <div class="glass absolute left-0 z-30 w-full max-w-md overflow-hidden rounded-xl border shadow-3 {placement === 'above' ? 'bottom-full mb-1.5' : 'mt-1.5'}" role="listbox" aria-label={t("attach.attach")} id="{uid}-list">
    {#each [{ key: "documents", name: t("attach.documents") }, { key: "files", name: t("attach.workspace") }] as group (group.key)}
      {@const rows = options.map((o, index) => ({ o, index })).filter((r) => r.o.group === group.key)}
      {#if rows.length > 0}
        <div role="group" aria-label={group.name} class="p-1.5">
          <p class="px-2.5 pt-1.5 pb-1 text-2xs font-semibold tracking-wider text-muted uppercase" aria-hidden="true">{group.name}</p>
          {#each rows as { o, index } (o.path)}
            <!-- The mouse chooses on press, before the box loses focus and closes the menu. -->
            <div
              role="option"
              tabindex="-1"
              aria-selected={index === active}
              id="{uid}-opt-{index}"
              onmousedown={(event) => { event.preventDefault(); void choose(o); }}
              onmouseenter={() => (active = index)}
              class="flex h-9 cursor-pointer items-center gap-2.5 rounded-lg px-2.5 text-md {index === active ? 'bg-accent-soft text-fg' : 'text-fg/90'}"
            >
              <Icon name={group.key === "documents" ? "file-text" : "code"} size={15} class={index === active ? "text-accent" : "text-faint"} />
              <span class="mono min-w-0 flex-1 truncate text-xs" title={o.folder ? `${o.folder}/${o.name}` : o.name}>{o.folder ? `${o.folder}/${o.name}` : o.name}</span>
              {#if index === active}<Kbd>↵</Kbd>{/if}
            </div>
          {/each}
        </div>
      {/if}
    {/each}
  </div>
{/if}
