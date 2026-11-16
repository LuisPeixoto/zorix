<script lang="ts">
  import { theme } from "../../stores/theme.svelte.ts";
  import { MODE_LABEL } from "../../theme.ts";
  import { Icon, IconButton } from "../../ui/index.ts";

  // On macOS the window controls sit on top of this bar (titleBarStyle Overlay), so leave room for them.
  const isMac =
    typeof navigator !== "undefined" && "__TAURI_INTERNALS__" in window && /Mac/i.test(navigator.platform || navigator.userAgent);
  const modeIcon = $derived(theme.mode === "light" ? "sun" : theme.mode === "dark" ? "moon" : "monitor");
</script>

<header
  data-tauri-drag-region
  class="flex h-13 shrink-0 items-center gap-3 border-b border-line bg-panel pr-3 {isMac ? 'pl-[84px]' : 'pl-4'}"
>
  <div data-tauri-drag-region class="flex items-center gap-2.5">
    <span class="brand-gradient grid size-7 place-items-center rounded-lg text-white shadow-[0_6px_16px_-6px_var(--accent)]">
      <Icon name="layers" size={15} stroke={2} />
    </span>
    <span data-tauri-drag-region class="text-lg font-semibold tracking-tight">ZORIX</span>
  </div>
  <div data-tauri-drag-region class="h-full flex-1"></div>
  <IconButton
    icon={modeIcon}
    label="Theme: {MODE_LABEL[theme.mode]}. Click to change."
    onclick={() => theme.cycle()}
  />
</header>
