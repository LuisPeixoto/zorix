// The pure part of the interface text (ADR-0040): finding a message, filling it in and choosing its plural. No state, no browser.

/** Values a message can hold in its `{placeholders}`. */
export type Args = Record<string, string | number | boolean | null | undefined>;

/** A message: one sentence, or its forms for each plural category (`other` is always there). A message with forms needs a `count`. */
export type Message = string | ({ other: string } & Partial<Record<"zero" | "one" | "two" | "few" | "many", string>>);

/** A set of messages by key. */
export type Catalogue = Readonly<Record<string, Message>>;

const PLACEHOLDER = /\{([A-Za-z_][A-Za-z0-9_]*)\}/g;

/** Put `args` in the places the sentence marks with `{name}`. A name with no value is left as it is, so a missing value shows. */
export function fill(text: string, args?: Args): string {
  if (!args) return text;
  return text.replace(PLACEHOLDER, (whole, name: string) => {
    const value = args[name];
    return value === undefined || value === null ? whole : String(value);
  });
}

/** The sentence of a message for these arguments, in this locale's plural rules. */
export function render(message: Message, locale: string, args?: Args): string {
  if (typeof message === "string") return fill(message, args);
  const count = Number(args?.count);
  if (!Number.isFinite(count)) return fill(message.other, args);
  const category = new Intl.PluralRules(locale).select(count);
  return fill(message[category as keyof typeof message] ?? message.other, args);
}

/**
 * The same sentence made longer and accented, so a layout that would cut a longer language shows it, and a string that did not go
 * through the catalogue stands out. Placeholders are kept whole. It is for tests and screenshots, never for a person.
 */
export function pseudo(text: string): string {
  const accents: Record<string, string> = { a: "á", e: "é", i: "í", o: "ö", u: "ü", c: "ç", n: "ñ", y: "ý", A: "Å", E: "É", I: "Î", O: "Ø", U: "Ü" };
  let out = "";
  let inPlaceholder = false;
  for (const ch of text) {
    if (ch === "{") inPlaceholder = true;
    out += inPlaceholder ? ch : (accents[ch] ?? ch);
    if (ch === "}") inPlaceholder = false;
  }
  // About 40 % longer, as German or Portuguese tend to be against English.
  const pad = "·".repeat(Math.ceil(text.replace(PLACEHOLDER, "").length * 0.4));
  return `[${out}${pad}]`;
}

/** Turn every message of a catalogue into its pseudo-locale form (plural forms too). */
export function pseudoCatalogue(source: Catalogue): Catalogue {
  const out: Record<string, Message> = {};
  for (const [key, message] of Object.entries(source)) {
    out[key] =
      typeof message === "string"
        ? pseudo(message)
        : (Object.fromEntries(Object.entries(message).map(([form, text]) => [form, pseudo(text as string)])) as Message);
  }
  return out;
}
