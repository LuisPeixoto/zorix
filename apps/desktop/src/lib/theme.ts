// Theme logic with no DOM in it, so it can be tested: which theme is shown, and what the toggle does next.

export type Mode = "light" | "dark" | "system";
export type Resolved = "light" | "dark";

export const STORAGE_KEY = "zorix.appearance";
export const DEFAULT_MODE: Mode = "light";

/** The native window background for each theme, so there is no flash while the page loads (matches `--bg`). */
export const WINDOW_BACKGROUND: Record<Resolved, string> = { light: "#f5f6fa", dark: "#14161d" };

export function isMode(value: unknown): value is Mode {
  return value === "light" || value === "dark" || value === "system";
}

/** What was saved, or the default when there is nothing usable. */
export function parseStored(text: string | null): Mode {
  if (!text) return DEFAULT_MODE;
  try {
    const value = JSON.parse(text) as { mode?: unknown };
    return isMode(value?.mode) ? value.mode : DEFAULT_MODE;
  } catch {
    return DEFAULT_MODE;
  }
}

export function serialize(mode: Mode): string {
  return JSON.stringify({ mode });
}

/** `system` follows the operating system; the others are what they say. */
export function resolve(mode: Mode, systemPrefersDark: boolean): Resolved {
  if (mode === "system") return systemPrefersDark ? "dark" : "light";
  return mode;
}

/** The toggle in the toolbar: light, then dark, then system, then light again. */
export function nextMode(mode: Mode): Mode {
  return mode === "light" ? "dark" : mode === "dark" ? "system" : "light";
}

export const MODE_LABEL: Record<Mode, string> = { light: "Light", dark: "Dark", system: "System" };
