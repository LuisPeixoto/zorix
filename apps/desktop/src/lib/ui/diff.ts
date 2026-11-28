// What changed between two texts, for reviewing a tool whose description or schema changed.

export type Part = { kind: "same" | "add" | "del"; text: string };

/** Longest common subsequence of two lists, as a table of lengths. Bounded: texts that are too big fall back to "all changed". */
function lcsTable(a: string[], b: string[]): number[][] {
  const table: number[][] = Array.from({ length: a.length + 1 }, () => new Array<number>(b.length + 1).fill(0));
  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      table[i][j] = a[i] === b[j] ? table[i + 1][j + 1] + 1 : Math.max(table[i + 1][j], table[i][j + 1]);
    }
  }
  return table;
}

const LIMIT = 2_000_000;

function diffLists(a: string[], b: string[], joiner: string): Part[] {
  if (a.length * b.length > LIMIT) {
    return [
      ...(a.length ? [{ kind: "del" as const, text: a.join(joiner) }] : []),
      ...(b.length ? [{ kind: "add" as const, text: b.join(joiner) }] : []),
    ];
  }
  const table = lcsTable(a, b);
  const parts: Part[] = [];
  const push = (kind: Part["kind"], text: string) => {
    const last = parts[parts.length - 1];
    if (last && last.kind === kind) last.text += joiner + text;
    else parts.push({ kind, text });
  };
  let i = 0;
  let j = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      push("same", a[i]);
      i++;
      j++;
    } else if (table[i + 1][j] >= table[i][j + 1]) {
      push("del", a[i++]);
    } else {
      push("add", b[j++]);
    }
  }
  while (i < a.length) push("del", a[i++]);
  while (j < b.length) push("add", b[j++]);
  return parts;
}

/** Line by line. Consecutive lines of the same kind are one part. */
export function diffLines(before: string, after: string): Part[] {
  return diffLists(before.split("\n"), after.split("\n"), "\n");
}

/** Word by word, for a sentence that changed in the middle. Spaces stay with the words around them. */
export function diffWords(before: string, after: string): Part[] {
  const split = (text: string) => text.match(/\s+|[^\s]+/g) ?? [];
  return diffLists(split(before), split(after), "");
}

/** How many lines or words were added and removed, for a one-line summary. */
export function summarize(parts: Part[]): { added: number; removed: number } {
  const count = (kind: Part["kind"]) =>
    parts.filter((p) => p.kind === kind).reduce((n, p) => n + (p.text.trim() ? p.text.split(/\n/).length : 0), 0);
  return { added: count("add"), removed: count("del") };
}

/** A unified patch (as git writes it) as lines to show: file headers, what was added and removed, and the context. */
export function patchLines(patch: string): { kind: "file" | "hunk" | "same" | "add" | "del"; text: string }[] {
  const lines = patch.split("\n");
  if (lines.at(-1) === "") lines.pop();
  let inHunk = false;
  return lines.flatMap((line): { kind: "file" | "hunk" | "same" | "add" | "del"; text: string }[] => {
    // Git's own header line, which is not something to translate.
    const file = /^diff --git a\/(.*) b\/.*$/.exec(line);
    if (file) {
      inHunk = false;
      return [{ kind: "file", text: file[1] }];
    }
    if (line.startsWith("@@")) {
      inHunk = true;
      return [{ kind: "hunk", text: line }];
    }
    // The header lines between the file and its first hunk say nothing more than the name.
    if (!inHunk) return [];
    if (line.startsWith("+")) return [{ kind: "add", text: line.slice(1) }];
    if (line.startsWith("-")) return [{ kind: "del", text: line.slice(1) }];
    return [{ kind: "same", text: line.startsWith(" ") ? line.slice(1) : line }];
  });
}
