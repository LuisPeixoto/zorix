// What a failed command tells the interface (ADR-0040). Pure: the node tests read it, and `api.ts` re-exports it.
import { knows, t } from "./i18n/index.ts";

/** What a command rejects with: a stable code, the values for its sentence, and the English sentence the command line would print. */
export interface UiError {
  code: string;
  args: Record<string, string>;
  message: string;
}

const isUiError = (value: unknown): value is UiError =>
  typeof value === "object" && value !== null && typeof (value as UiError).code === "string" && typeof (value as UiError).message === "string";

/** Turn whatever an invoke rejected with into a sentence in the active language. A code this interface does not know shows its English sentence. */
export function errorText(error: unknown): string {
  if (isUiError(error)) {
    const key = `reason.${error.code}`;
    return knows(key) ? t(key, error.args) : error.message;
  }
  return typeof error === "string" ? error : error instanceof Error ? error.message : JSON.stringify(error);
}
