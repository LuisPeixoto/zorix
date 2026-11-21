<script lang="ts">
  import { inbox } from "../../stores/inbox.svelte.ts";
  import { store } from "../../stores/runs.svelte.ts";
  import { theme } from "../../stores/theme.svelte.ts";
  import { MODE_LABEL } from "../../theme.ts";
  import { BrandMark, Icon, IconButton, Kbd } from "../../ui/index.ts";

  // On macOS the window controls sit on top of this bar (titleBarStyle Overlay), so leave room for them.
  const isMac =
    typeof navigator !== "undefined" && "__TAURI_INTERNALS__" in window && /Mac/i.test(navigator.platform || navigator.userAgent);
  const modeIcon = $derived(theme.mode === "light" ? "sun" : theme.mode === "dark" ? "moon" : "monitor");
  const shortcut = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform || navigator.userAgent) ? "⌘K" : "Ctrl K";
</script>

<header
  data-tauri-drag-region
  class="glass relative z-30 flex h-13 shrink-0 items-center gap-3 border-b pr-3 {isMac ? 'pl-[84px]' : 'pl-4'}"
>
  <div data-tauri-drag-region class="flex items-center gap-2.5">
    <BrandMark wordmark />
    {#if store.info?.version}<span class="mono rounded-full border border-line bg-panel px-2 text-xs text-muted">{store.info.version}</span>{/if}
  </div>
  <div data-tauri-drag-region class="flex h-full flex-1 items-center justify-center">
    <button
      type="button"
      onclick={() => (store.palette = true)}
      aria-label="Open the command palette"
      class="flex h-8 w-full max-w-md items-center gap-2 rounded-lg border border-line bg-panel-2/70 px-3 text-base text-faint transition duration-150 hover:border-line-strong hover:bg-panel hover:text-muted"
    >
      <Icon name="search" size={14} />
      <span class="flex-1 text-left">Search runs, workflows, agents and tasks</span>
      <Kbd>{shortcut}</Kbd>
    </button>
  </div>
  <button
    type="button"
    onclick={() => store.showInbox()}
    class="relative grid size-8 place-items-center rounded-lg text-muted transition duration-150 hover:bg-hover hover:text-fg"
    aria-label={inbox.count > 0 ? `Inbox: ${inbox.count} waiting for you` : "Inbox: nothing is waiting for you"}
  >
    <Icon name="bell" size={16} />
    {#if inbox.count > 0}<span class="tnum absolute top-0.5 right-0.5 grid h-3.5 min-w-3.5 place-items-center rounded-full bg-warn px-1 text-2xs font-bold text-bg">{inbox.count}</span>{/if}
  </button>
  <IconButton
    icon={modeIcon}
    label="Theme: {MODE_LABEL[theme.mode]}. Click to change."
    onclick={() => theme.cycle()}
  />
</header>
