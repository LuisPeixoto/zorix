/// <reference types="svelte" />
/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** `design`, `providers` or `run` (the newest run): open that screen at start (screenshots and reviews). */
  readonly VITE_START_VIEW?: string;
  /** `light`, `dark` or `system`: force a theme for this build without saving it. */
  readonly VITE_START_THEME?: string;
}
