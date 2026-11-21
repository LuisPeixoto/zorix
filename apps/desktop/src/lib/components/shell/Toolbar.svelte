<script lang="ts">
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
  </div>
  <div data-tauri-drag-region class="flex h-full flex-1 items-center justify-center">
    <button
      type="button"
      onclick={() => (store.palette = true)}
      aria-label="Open the command palette"
      class="flex h-8 w-full max-w-md items-center gap-2 rounded-lg border border-line bg-panel-2/70 px-3 text-base text-faint transition duration-150 hover:border-line-strong hover:bg-panel hover:text-muted"
    >
      <Icon name="search" size={14} />
      <span class="flex-1 text-left">Search runs, servers and actions</span>
      <Kbd>{shortcut}</Kbd>
    </button>
  </div>
  <IconButton
    icon={modeIcon}
    label="Theme: {MODE_LABEL[theme.mode]}. Click to change."
    onclick={() => theme.cycle()}
  />
</header>
