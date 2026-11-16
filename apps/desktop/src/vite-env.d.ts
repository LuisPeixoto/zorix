/// <reference types="svelte" />
/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** `design`: open the design system at start (screenshots and reviews). */
  readonly VITE_START_VIEW?: string;
  /** `light`, `dark` or `system`: force a theme for this build without saving it. */
  readonly VITE_START_THEME?: string;
}
