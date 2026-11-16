// The person's choice of theme: light (the default), dark, or follow the system. Remembered between runs.
import { getCurrentWindow } from "@tauri-apps/api/window";

import {
  type Mode,
  type Resolved,
  STORAGE_KEY,
  WINDOW_BACKGROUND,
  nextMode,
  parseStored,
  resolve,
  serialize,
} from "../theme.ts";

class ThemeStore {
  mode = $state<Mode>("light");
  systemDark = $state(false);
  #started = false;

  get resolved(): Resolved {
    return resolve(this.mode, this.systemDark);
  }

  /** Read the saved choice, follow the system and paint. Safe to call more than once. */
  init(): void {
    if (this.#started) return;
    this.#started = true;
    try {
      this.mode = parseStored(localStorage.getItem(STORAGE_KEY));
    } catch {
      /* storage can be unavailable; the default stands */
    }
    if (typeof matchMedia === "function") {
      const query = matchMedia("(prefers-color-scheme: dark)");
      this.systemDark = query.matches;
      query.addEventListener("change", (event) => {
        this.systemDark = event.matches;
        this.#apply();
      });
    }
    this.#apply();
  }

  set(mode: Mode): void {
    this.mode = mode;
    try {
      localStorage.setItem(STORAGE_KEY, serialize(mode));
    } catch {
      /* the choice still applies for this run */
    }
    this.#apply();
  }

  /** The toolbar button: light, dark, system. */
  cycle(): void {
    this.set(nextMode(this.mode));
  }

  #apply(): void {
    const theme = this.resolved;
    document.documentElement.dataset.theme = theme;
    document.documentElement.style.colorScheme = theme;
    // The window behind the page, so resizing never shows the wrong colour. Not available in a plain browser.
    void getCurrentWindow()
      .setBackgroundColor(WINDOW_BACKGROUND[theme])
      .catch(() => {});
  }
}

export const theme = new ThemeStore();
