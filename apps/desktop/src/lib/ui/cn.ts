/** Anything a `class` attribute accepts: strings, arrays, and falsy values that mean "nothing". */
export type ClassPart = string | number | bigint | boolean | null | undefined | ClassPart[] | { [key: string]: unknown };

/** Join class names, skipping the empty ones. Arrays are flattened; an object keeps the keys whose value is truthy. */
export function cn(...parts: ClassPart[]): string {
  const out: string[] = [];
  const add = (part: ClassPart): void => {
    if (!part && part !== 0) return;
    if (Array.isArray(part)) part.forEach(add);
    else if (typeof part === "object") for (const [key, on] of Object.entries(part)) on && out.push(key);
    else if (part !== true) out.push(String(part));
  };
  parts.forEach(add);
  return out.join(" ");
}
