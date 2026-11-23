// The source catalogue of the interface (ADR-0040): English. A message is a sentence with `{placeholders}`, or its plural forms
// (`one`, `other`) for a message that counts something. Keys are grouped by the part of the app that shows them.
import type { Message } from "./core.ts";

export const en = {
  "common.cancel": "Cancel",
} as const satisfies Record<string, Message>;
