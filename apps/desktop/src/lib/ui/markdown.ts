// A small, safe Markdown reader for what models write: headings, paragraphs, lists, quotes, code, tables, links and the
// usual emphasis. It builds a tree and never HTML, so nothing a model says can become markup. Links keep only http, https
// and mailto addresses; images are shown as their description; raw HTML is text.

export type Inline =
  | { t: "text"; text: string }
  | { t: "code"; text: string }
  | { t: "strong"; children: Inline[] }
  | { t: "em"; children: Inline[] }
  | { t: "del"; children: Inline[] }
  | { t: "link"; href: string; children: Inline[] }
  | { t: "br" };

export type Block =
  | { t: "heading"; level: number; children: Inline[] }
  | { t: "p"; children: Inline[] }
  | { t: "code"; lang: string; text: string }
  | { t: "quote"; children: Block[] }
  | { t: "list"; ordered: boolean; start: number; items: Block[][] }
  | { t: "table"; head: Inline[][]; align: ("left" | "right" | "center" | null)[]; rows: Inline[][][] }
  | { t: "hr" };

/** The longest text that is read; the rest is ignored (a runaway answer should not freeze the screen). */
export const MAX_CHARS = 200_000;
const MAX_DEPTH = 6;

/** An address that is safe to open: http, https or mailto. Anything else (javascript:, data:, file:…) is refused. */
export function safeHref(raw: string): string | null {
  const href = raw.trim();
  if (!href || /[\u0000-\u001f\u007f\s]/.test(href)) return null;
  if (/^(https?:\/\/|mailto:)/i.test(href)) {
    try {
      const url = new URL(href);
      return url.protocol === "http:" || url.protocol === "https:" || url.protocol === "mailto:" ? href : null;
    } catch {
      return null;
    }
  }
  return null;
}

// ---------------------------------------------------------------------------------------------------------------------
// Inline

function plain(text: string): Inline[] {
  return text ? [{ t: "text", text }] : [];
}

function push(out: Inline[], node: Inline) {
  const last = out[out.length - 1];
  if (node.t === "text" && last?.t === "text") last.text += node.text;
  else out.push(node);
}

/** How far a closing mark may be from its opening one. Beyond it the opening mark is only text. */
const MAX_SPAN = 4000;

/** Find `delimiter` after `from`, not escaped and not directly after a space (a closing mark hugs its text). */
function closing(text: string, delimiter: string, from: number): number {
  let at = from;
  while ((at = text.indexOf(delimiter, at)) !== -1) {
    if (at - from > MAX_SPAN) return -1;
    const before = text[at - 1];
    if (before !== "\\" && before !== " " && before !== "\n" && at > from) return at;
    at += 1;
  }
  return -1;
}

const STICKY = {
  ticks: /`+/y,
  image: /!\[([^\]\n]{0,300})\]\(((?:[^()\s]|\([^()\s]*\)){0,2000})(?:\s+"[^"]{0,300}")?\)/y,
  link: /\[([^\]\n]{1,300})\]\(((?:[^()\s]|\([^()\s]*\)){0,2000})(?:\s+"[^"]{0,300}")?\)/y,
  angle: /<((?:https?:\/\/|mailto:)[^\s<>]+)>/iy,
  bare: /https?:\/\/[^\s<>]+/iy,
};

function match(re: RegExp, text: string, at: number): RegExpExecArray | null {
  re.lastIndex = at;
  return re.exec(text);
}

const MARKS = [
  ["**", "strong"],
  ["__", "strong"],
  ["~~", "del"],
  ["*", "em"],
  ["_", "em"],
] as const;

export function parseInline(text: string, depth = 0): Inline[] {
  if (depth >= MAX_DEPTH) return plain(text);
  const out: Inline[] = [];
  let i = 0;
  while (i < text.length) {
    const c = text[i];
    if (c === "\\" && i + 1 < text.length && /[\\`*_{}[\]()#+\-.!|~<>]/.test(text[i + 1])) {
      push(out, { t: "text", text: text[i + 1] });
      i += 2;
      continue;
    }
    if (c === "\n") {
      out.push({ t: "br" });
      i += 1;
      continue;
    }
    if (c === "`") {
      const ticks = match(STICKY.ticks, text, i)![0];
      const end = text.indexOf(ticks, i + ticks.length);
      if (end !== -1 && end - i <= MAX_SPAN) {
        push(out, { t: "code", text: text.slice(i + ticks.length, end).replace(/^ (.*) $/s, "$1") });
        i = end + ticks.length;
        continue;
      }
      push(out, { t: "text", text: ticks });
      i += ticks.length;
      continue;
    }
    if (c === "!" && text[i + 1] === "[") {
      const m = match(STICKY.image, text, i);
      if (m) {
        push(out, { t: "text", text: m[1] || "image" });
        i += m[0].length;
        continue;
      }
    }
    if (c === "[") {
      const m = match(STICKY.link, text, i);
      if (m) {
        const href = safeHref(m[2]);
        const children = parseInline(m[1], depth + 1);
        if (href) out.push({ t: "link", href, children });
        else for (const child of children) push(out, child);
        i += m[0].length;
        continue;
      }
    }
    if (c === "<") {
      const m = match(STICKY.angle, text, i);
      const href = m ? safeHref(m[1]) : null;
      if (m && href) {
        out.push({ t: "link", href, children: plain(m[1]) });
        i += m[0].length;
        continue;
      }
    }
    if (c === "h" && (i === 0 || /[\s(]/.test(text[i - 1]))) {
      const m = match(STICKY.bare, text, i);
      if (m) {
        const url = m[0].replace(/[.,;:!?)\]]+$/, "");
        const href = safeHref(url);
        if (href) {
          out.push({ t: "link", href, children: plain(url) });
          i += url.length;
          continue;
        }
      }
    }
    let matched = false;
    for (const [mark, kind] of MARKS) {
      if (!text.startsWith(mark, i)) continue;
      // `_` inside a word (snake_case) is not emphasis.
      if (mark[0] === "_" && i > 0 && /\w/.test(text[i - 1])) continue;
      if (i + mark.length >= text.length || text[i + mark.length] === " ") continue;
      const end = closing(text, mark, i + mark.length);
      if (end === -1) continue;
      if (mark[0] === "_" && /\w/.test(text[end + mark.length] ?? "")) continue;
      out.push({ t: kind, children: parseInline(text.slice(i + mark.length, end), depth + 1) });
      i = end + mark.length;
      matched = true;
      break;
    }
    if (matched) continue;
    push(out, { t: "text", text: c });
    i += 1;
  }
  return out;
}

// ---------------------------------------------------------------------------------------------------------------------
// Blocks

const FENCE = /^(\s{0,3})(`{3,}|~{3,})\s*([\w+#.-]*)[^`]*$/;
const HEADING = /^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$/;
const HR = /^\s{0,3}([-*_])(?:\s*\1){2,}\s*$/;
const BULLET = /^(\s*)([-*+])\s+(.*)$/;
const ORDERED = /^(\s*)(\d{1,9})[.)]\s+(.*)$/;
const QUOTE = /^\s{0,3}>\s?(.*)$/;
const TABLE_SEPARATOR = /^\s*\|?\s*:?-{1,}:?\s*(\|\s*:?-{1,}:?\s*)*\|?\s*$/;

function splitRow(line: string): string[] {
  let body = line.trim();
  if (body.startsWith("|")) body = body.slice(1);
  if (body.endsWith("|") && !body.endsWith("\\|")) body = body.slice(0, -1);
  return body.split(/(?<!\\)\|/).map((cell) => cell.trim().replace(/\\\|/g, "|"));
}

function startsBlock(line: string): boolean {
  return FENCE.test(line) || HEADING.test(line) || HR.test(line) || QUOTE.test(line) || BULLET.test(line) || ORDERED.test(line);
}

export function parseBlocks(source: string, depth = 0): Block[] {
  const lines = source.replace(/\r\n?/g, "\n").split("\n");
  const blocks: Block[] = [];
  if (depth >= MAX_DEPTH) return source.trim() ? [{ t: "p", children: plain(source) }] : [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (!line.trim()) {
      i += 1;
      continue;
    }
    const fence = FENCE.exec(line);
    if (fence) {
      const [, indent, marks, lang] = fence;
      const body: string[] = [];
      i += 1;
      while (i < lines.length && !new RegExp(`^\\s{0,3}${marks[0]}{${marks.length},}\\s*$`).test(lines[i])) {
        body.push(lines[i].startsWith(indent) ? lines[i].slice(indent.length) : lines[i]);
        i += 1;
      }
      i += 1; // the closing fence (or the end of the text: an unclosed fence runs to the end)
      blocks.push({ t: "code", lang: lang || "", text: body.join("\n") });
      continue;
    }
    const heading = HEADING.exec(line);
    if (heading) {
      blocks.push({ t: "heading", level: heading[1].length, children: parseInline(heading[2]) });
      i += 1;
      continue;
    }
    if (HR.test(line)) {
      blocks.push({ t: "hr" });
      i += 1;
      continue;
    }
    if (QUOTE.test(line)) {
      const body: string[] = [];
      while (i < lines.length && QUOTE.test(lines[i])) {
        body.push(QUOTE.exec(lines[i])![1]);
        i += 1;
      }
      blocks.push({ t: "quote", children: parseBlocks(body.join("\n"), depth + 1) });
      continue;
    }
    const bullet = BULLET.exec(line);
    const ordered = bullet ? null : ORDERED.exec(line);
    if (bullet || ordered) {
      const m = (bullet ?? ordered)!;
      const isOrdered = !bullet;
      const start = isOrdered ? Number(m[2]) : 1;
      const baseIndent = m[1].length;
      const items: Block[][] = [];
      while (i < lines.length) {
        const item = (isOrdered ? ORDERED : BULLET).exec(lines[i]);
        if (!item || item[1].length !== baseIndent) break;
        const width = item[0].length - item[3].length;
        const body = [item[3]];
        i += 1;
        while (i < lines.length) {
          const next = lines[i];
          if (!next.trim()) {
            // A blank line continues the item only when the next line is indented under it.
            if (i + 1 < lines.length && lines[i + 1].startsWith(" ".repeat(Math.min(width, baseIndent + 2)))) {
              body.push("");
              i += 1;
              continue;
            }
            break;
          }
          if (next.length - next.trimStart().length > baseIndent) {
            body.push(next.slice(Math.min(next.length - next.trimStart().length, width)));
            i += 1;
            continue;
          }
          break;
        }
        items.push(parseBlocks(body.join("\n"), depth + 1));
      }
      blocks.push({ t: "list", ordered: isOrdered, start, items });
      continue;
    }
    if (line.includes("|") && i + 1 < lines.length && TABLE_SEPARATOR.test(lines[i + 1]) && lines[i + 1].includes("-")) {
      const head = splitRow(line);
      const separators = splitRow(lines[i + 1]);
      if (head.length === separators.length) {
        const align = separators.map((cell) => {
          const left = cell.startsWith(":");
          const right = cell.endsWith(":");
          return left && right ? "center" : right ? "right" : left ? "left" : null;
        });
        i += 2;
        const rows: Inline[][][] = [];
        while (i < lines.length && lines[i].trim() && lines[i].includes("|")) {
          const cells = splitRow(lines[i]);
          rows.push(head.map((_, k) => parseInline(cells[k] ?? "")));
          i += 1;
        }
        blocks.push({ t: "table", head: head.map((h) => parseInline(h)), align, rows });
        continue;
      }
    }
    const para: string[] = [line.trimEnd()];
    i += 1;
    while (i < lines.length && lines[i].trim() && !startsBlock(lines[i])) {
      para.push(lines[i].trimEnd());
      i += 1;
    }
    blocks.push({ t: "p", children: parseInline(para.join("\n").trim()) });
  }
  return blocks;
}

/** Read Markdown into a tree. Text past [`MAX_CHARS`] is dropped. */
export function parseMarkdown(source: string): Block[] {
  return parseBlocks(source.length > MAX_CHARS ? source.slice(0, MAX_CHARS) : source);
}

/** The plain text of inline nodes (for a title or an accessible label). */
export function textOf(nodes: Inline[]): string {
  return nodes
    .map((n) => (n.t === "text" || n.t === "code" ? n.text : n.t === "br" ? " " : "children" in n ? textOf(n.children) : ""))
    .join("");
}
