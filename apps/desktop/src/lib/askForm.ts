// What a person typed into a server's form, as the values the server gets. The backend checks them again.
import type { AskField } from "./api.ts";

export type Typed = Record<string, string | boolean>;

/** Where each field starts: empty text, nothing chosen, unticked. */
export function emptyForm(fields: readonly AskField[]): Typed {
  return Object.fromEntries(fields.map((f) => [f.name, f.kind.type === "boolean" ? false : ""]));
}

/** The values to send, or the first field that is missing or does not fit. */
export function formValues(fields: readonly AskField[], typed: Typed): { values: Record<string, unknown>; problem: AskField | null } {
  const values: Record<string, unknown> = {};
  for (const field of fields) {
    const raw = typed[field.name];
    if (field.kind.type === "boolean") {
      values[field.name] = raw === true;
      continue;
    }
    const text = typeof raw === "string" ? raw : "";
    if (text.trim() === "") {
      if (field.required) return { values, problem: field };
      continue;
    }
    if (field.kind.type === "number" || field.kind.type === "integer") {
      const number = Number(text.trim());
      if (!Number.isFinite(number) || (field.kind.type === "integer" && !Number.isInteger(number))) return { values, problem: field };
      values[field.name] = number;
    } else if (field.kind.type === "choice") {
      if (!field.kind.options.includes(text)) return { values, problem: field };
      values[field.name] = text;
    } else {
      values[field.name] = text;
    }
  }
  return { values, problem: null };
}

/** The host a link leads to, shown apart so it is read before anything is opened. */
export function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}
