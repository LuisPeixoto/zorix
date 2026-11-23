// The interface's text and formats (ADR-0040). `t("runs.title")` gives a message of the active locale; every sentence a person reads is in a
// catalogue, none in a component. English is the source and the fallback for any key another language lacks.
//
// The locale is read once, when the app starts, and changing it reloads the window: nothing here is reactive, so any module may call `t`
// while it is being set up, and the node tests of the helpers run without a browser.
import { type Args, type Catalogue, type Message, pseudoCatalogue, render } from "./core.ts";
import { type Formats, formatsFor } from "./format.ts";
import { en } from "./messages.en.ts";

export type { Args, Catalogue, Message } from "./core.ts";
export type { Formats } from "./format.ts";

/** Every key of the source catalogue. */
export type Key = keyof typeof en;

/** The locales the interface has: English, and the pseudo-locale that tests and screenshots use to find text that does not fit. */
export const LOCALES = ["en", "en-XA"] as const;
export type Locale = (typeof LOCALES)[number];

const STORAGE_KEY = "zorix.locale";

function chosen(): Locale {
  // A build for screenshots can ask for a locale (`VITE_START_LOCALE=en-XA`); a person's choice is remembered in storage.
  const asked = (import.meta as { env?: Record<string, string | undefined> }).env?.VITE_START_LOCALE;
  if ((LOCALES as readonly string[]).includes(asked ?? "")) return asked as Locale;
  try {
    const stored = globalThis.localStorage?.getItem(STORAGE_KEY);
    if ((LOCALES as readonly string[]).includes(stored ?? "")) return stored as Locale;
  } catch {
    // A browser with storage blocked: the default is fine.
  }
  return "en";
}

let current: Locale = chosen();
let catalogue: Catalogue = current === "en-XA" ? pseudoCatalogue(en) : en;
let formats: Formats = formatsFor(current);
announce();

/** The page says which language it is in, for screen readers and the browser's own hyphenation. */
function announce(): void {
  try {
    if (typeof document !== "undefined") document.documentElement.lang = current === "en-XA" ? "en" : current;
  } catch {
    // No document (the tests): nothing to say.
  }
}

/** The active locale. */
export const locale = (): Locale => current;

/** Use a locale in this program (tests, and the start of the app). The app itself changes language by storing it and reloading. */
export function useLocale(next: Locale): void {
  current = next;
  catalogue = next === "en-XA" ? pseudoCatalogue(en) : en;
  formats = formatsFor(next);
  announce();
}

/** Remember a locale for the next start. */
export function rememberLocale(next: Locale): void {
  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, next);
  } catch {
    // Not remembered; the window still shows it until it reloads.
  }
}

/** The message of a key, filled in. A key with plural forms needs a `count`. */
export function t(key: Key, args?: Args): string {
  const message: Message | undefined = catalogue[key] ?? en[key];
  return message === undefined ? key : render(message, current === "en-XA" ? "en" : current, args);
}

/** Formatters for the active locale. */
export const fmt: Formats = new Proxy({} as Formats, {
  get: (_target, name: string) => (formats as unknown as Record<string, unknown>)[name],
});
