// What the form does with the files a run is given (ADR-0049): the `@` that opens the menu, where the name goes, and how the list of files grows.
// The rules that matter (what may be attached) are the program's, checked again when the run starts; this only keeps the form honest.

/** The most files a run can be given. The program refuses more. */
export const MAX_ATTACHED = 10;

/** A file on the form. */
export interface AttachedFile {
  path: string;
  name: string;
  bytes: number;
  /** It is not inside the workspace folder. A person chose it, so it is allowed; the form says so. */
  outside: boolean;
}

/** What the program said about a file that was chosen. */
export interface CheckedFile {
  path: string;
  name: string;
  bytes: number;
  problem: string | null;
  outside_workspace: boolean;
}

/** An `@word` being typed: where it starts (at the `@`), where the caret is, and what follows the `@`. */
export interface Mention {
  start: number;
  end: number;
  query: string;
}

/** The mention the caret is in the middle of, if any. An `@` counts at the start of the text or after a space, a line break or an opening bracket (not in an address). */
export function mentionAt(text: string, caret: number): Mention | null {
  const before = text.slice(0, caret);
  const found = /(^|[\s(])@([^\s@]*)$/.exec(before);
  if (!found) return null;
  return { start: before.length - found[2].length - 1, end: caret, query: found[2] };
}

/** The text with the mention replaced by `@name ` and where the caret goes. */
export function insertMention(text: string, mention: Mention, name: string): { text: string; caret: number } {
  const inserted = `@${name} `;
  const after = text.slice(mention.end);
  return { text: text.slice(0, mention.start) + inserted + (after.startsWith(" ") ? after.slice(1) : after), caret: mention.start + inserted.length };
}

/** A rough number of characters of text in the files, for the estimate: a PDF holds far less text than bytes. */
export function estimatedChars(files: AttachedFile[]): number {
  return files.reduce((sum, f) => sum + (/\.pdf$/i.test(f.name) ? Math.round(f.bytes / 8) : f.bytes), 0);
}

/** The files after adding what was just checked: the ones that can be attached are added (a path already there is not added twice), up to the limit. */
export function addChecked(
  current: AttachedFile[],
  checked: CheckedFile[],
): { files: AttachedFile[]; refused: { name: string; problem: string }[]; overLimit: boolean } {
  const files = [...current];
  const refused: { name: string; problem: string }[] = [];
  let overLimit = false;
  for (const item of checked) {
    if (item.problem) {
      refused.push({ name: item.name, problem: item.problem });
      continue;
    }
    if (files.some((f) => f.path === item.path)) continue;
    if (files.length >= MAX_ATTACHED) {
      overLimit = true;
      continue;
    }
    files.push({ path: item.path, name: item.name, bytes: item.bytes, outside: item.outside_workspace });
  }
  return { files, refused, overLimit };
}

/** The files with their "outside the workspace" flag brought up to date from a new check. The same array comes back when nothing changed, so that storing it does not wake what follows it. */
export function withOutsideFlags(files: AttachedFile[], checked: CheckedFile[]): AttachedFile[] {
  const byPath = new Map(checked.map((c) => [c.path, c]));
  let changed = false;
  const next = files.map((f) => {
    const outside = byPath.get(f.path)?.outside_workspace ?? f.outside;
    if (outside === f.outside) return f;
    changed = true;
    return { ...f, outside };
  });
  return changed ? next : files;
}
