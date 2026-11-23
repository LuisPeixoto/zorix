import { t as tr } from "../i18n/index.ts";
// A tiny syntax highlighter for the code ZORIX shows people: SQL, JSON, JavaScript and Python. It turns text into tokens;
// the component decides how to paint them. No network, no dependency, and never HTML: the text is always rendered as text.

export type TokenKind = "kw" | "str" | "num" | "com" | "fn" | "punct" | "plain";
export type Token = { kind: TokenKind; text: string };
export type Language = "sql" | "json" | "js" | "py" | "text";

const SQL_KEYWORDS = new Set(
  tr("ui.highlight.selectFromWhereAnd").split(" "),
);
const JS_KEYWORDS = new Set(
  tr("ui.highlight.constLetVarFunction").split(" "),
);
const PY_KEYWORDS = new Set(
  tr("ui.highlight.defReturnIfElif").split(" "),
);

type Rule = { kind: TokenKind; re: RegExp };

function scanner(rules: Rule[], keywords: Set<string>, caseInsensitive: boolean) {
  return (code: string): Token[] => {
    const tokens: Token[] = [];
    let i = 0;
    let plain = "";
    const flush = () => {
      if (plain) tokens.push({ kind: "plain", text: plain });
      plain = "";
    };
    while (i < code.length) {
      let matched = false;
      for (const rule of rules) {
        rule.re.lastIndex = i;
        const m = rule.re.exec(code);
        if (m && m.index === i && m[0].length > 0) {
          flush();
          let kind = rule.kind;
          if (kind === "plain") {
            // A word: a keyword, a function name (followed by a parenthesis), or just text.
            const word = caseInsensitive ? m[0].toLowerCase() : m[0];
            if (keywords.has(word)) kind = "kw";
            else if (code[i + m[0].length] === "(") kind = "fn";
          }
          tokens.push({ kind, text: m[0] });
          i += m[0].length;
          matched = true;
          break;
        }
      }
      if (!matched) {
        plain += code[i];
        i += 1;
      }
    }
    flush();
    return tokens;
  };
}

const WORD = /[A-Za-z_][A-Za-z0-9_]*/y;
const NUMBER = /\b\d+(?:\.\d+)?(?:[eE][+-]?\d+)?\b/y;
const PUNCT = /[{}()[\],;:.=<>!+\-*/%|&^~?@]+/y;

const sql = scanner(
  [
    { kind: "com", re: /--[^\n]*|\/\*[\s\S]*?\*\//y },
    { kind: "str", re: /'(?:[^']|'')*'?|"(?:[^"]|"")*"?/y },
    { kind: "num", re: NUMBER },
    { kind: "plain", re: WORD },
    { kind: "punct", re: PUNCT },
  ],
  SQL_KEYWORDS,
  true,
);
const js = scanner(
  [
    { kind: "com", re: /\/\/[^\n]*|\/\*[\s\S]*?\*\//y },
    { kind: "str", re: /`(?:\\[\s\S]|[^`\\])*`?|'(?:\\.|[^'\\\n])*'?|"(?:\\.|[^"\\\n])*"?/y },
    { kind: "num", re: NUMBER },
    { kind: "plain", re: WORD },
    { kind: "punct", re: PUNCT },
  ],
  JS_KEYWORDS,
  false,
);
const py = scanner(
  [
    { kind: "com", re: /#[^\n]*/y },
    { kind: "str", re: /"""[\s\S]*?(?:"""|$)|'''[\s\S]*?(?:'''|$)|'(?:\\.|[^'\\\n])*'?|"(?:\\.|[^"\\\n])*"?/y },
    { kind: "num", re: NUMBER },
    { kind: "plain", re: WORD },
    { kind: "punct", re: PUNCT },
  ],
  PY_KEYWORDS,
  false,
);
const json = scanner(
  [
    { kind: "str", re: /"(?:\\.|[^"\\\n])*"?/y },
    { kind: "num", re: /-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/y },
    { kind: "plain", re: /true|false|null/y },
    { kind: "punct", re: /[{}[\],:]+/y },
  ],
  new Set(["true", "false", "null"]),
  false,
);

/** Split `code` into tokens. The tokens always join back to exactly `code`. */
export function highlight(code: string, language: Language): Token[] {
  switch (language) {
    case "sql":
      return sql(code);
    case "js":
      return js(code);
    case "py":
      return py(code);
    case "json":
      return json(code);
    default:
      return code ? [{ kind: "plain", text: code }] : [];
  }
}

/** A JSON object's keys read better in a different colour than its string values. */
export function markJsonKeys(tokens: Token[]): Token[] {
  return tokens.map((token, index) => {
    if (token.kind !== "str") return token;
    const next = tokens.slice(index + 1).find((t) => t.text.trim() !== "");
    return next && next.text.startsWith(":") ? { kind: "fn", text: token.text } : token;
  });
}
