// Runs before the interface is mounted, so the first paint already has the right colours.
import { DEFAULT_MODE, STORAGE_KEY, parseStored, resolve } from "./lib/theme.ts";
import { t } from "./lib/i18n/index.ts";

try {
  const mode = parseStored(localStorage.getItem(STORAGE_KEY));
  const dark = typeof matchMedia === "function" && matchMedia(t("early.prefersColorSchemeDark")).matches;
  const theme = resolve(mode, dark);
  document.documentElement.dataset.theme = theme;
  document.documentElement.style.colorScheme = theme;
} catch {
  document.documentElement.dataset.theme = DEFAULT_MODE;
}
