// Finds the text a person reads that is written in the code instead of the catalogue (ADR-0040). Used by tools/check-text.mjs.
//
// In a .svelte file: text between tags, and the values of the attributes that name something a person reads (aria-label, title,
// placeholder, label, hint ...), as plain text or as a string inside an expression. In a .ts file and in a script: string and template
// literals that read as prose. A literal is prose when it has two words, or one word that starts with a capital: "Save", "Not found",
// `${n} tasks`. Identifiers, class names, paths, urls and keyboard glyphs are not.
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const require = createRequire(join(root, "apps/desktop/package.json"));
const { parse } = require("svelte/compiler");
const ts = require("typescript");

/** Attributes whose text a person reads. */
export const READ_ATTRIBUTES = new Set([
  "aria-label", "aria-description", "aria-placeholder", "aria-roledescription", "aria-valuetext", "title", "placeholder", "alt", "label",
  "hint", "description", "subtitle", "text", "tooltip", "caption", "heading", "message", "help", "legend", "summary", "empty", "emptyText",
  "confirm", "cancel", "submit", "unit", "suffix", "prefix", "badge", "note",
]);

const LETTERS = /\p{L}{2,}/u;

/** Does this string read as something a person reads? */
export function isProse(text) {
  const t = text.trim();
  if (!LETTERS.test(t)) return false;
  if (/^(https?:|\/|\.\/|#|--|[a-z]+:\/\/)/.test(t)) return false; // an address, a path, a token
  if (/^[\w.\-:/@%$#{}()[\]<>=+*,;|&^!~?'"` ]*$/.test(t) && !/\s/.test(t) && !/^[A-Z]/.test(t)) return false; // one lower-case word: an identifier
  if (!/\s/.test(t) && /^[a-z0-9_.\-/:]+$/i.test(t) && !/^[A-Z][a-z]/.test(t)) return false; // snake, kebab, dotted, upper-case constants
  if (/^[A-Z0-9_]+$/.test(t)) return false;
  if (/^[a-z]+([A-Z][a-z]+)+$/.test(t)) return false; // camelCase
  return true;
}

const lineOf = (source, index) => source.slice(0, index).split("\n").length;

function walk(node, visit, parents = []) {
  if (!node || typeof node !== "object") return;
  if (Array.isArray(node)) {
    for (const child of node) walk(child, visit, parents);
    return;
  }
  if (typeof node.type === "string") {
    visit(node, parents);
    parents = [...parents, node];
  }
  for (const key of Object.keys(node)) {
    if (key === "parent" || key === "metadata") continue;
    walk(node[key], visit, parents);
  }
}

/** String literals inside a JS expression (as acorn/estree nodes) that read as prose, skipping comparison operands. */
function proseInExpression(expression, offset = 0) {
  const found = [];
  walk(expression, (node, parents) => {
    const parent = parents[parents.length - 1];
    const inComparison = parent?.type === "BinaryExpression" && ["===", "!==", "==", "!=", "in"].includes(parent.operator);
    const isKey = parent?.type === "Property" && parent.key === node && !parent.computed;
    const isTest = parent?.type === "SwitchCase" && parent.test === node;
    if (node.type === "Literal" && typeof node.value === "string" && !inComparison && !isKey && !isTest && isProse(node.value)) {
      found.push({ start: node.start + offset, end: node.end + offset, text: node.value });
    }
    if (node.type === "TemplateElement" && !inComparison && isProse(node.value.cooked ?? node.value.raw)) {
      found.push({ start: node.start + offset, end: node.end + offset, text: node.value.cooked ?? node.value.raw });
    }
  });
  return found;
}

/** Findings in a .svelte file: `{ line, kind, text }`. */
export function scanSvelte(source) {
  const ast = parse(source, { modern: true });
  const out = [];
  const push = (kind, start, text) => out.push({ line: lineOf(source, start), kind, text: text.replace(/\s+/g, " ").trim() });
  walk(ast.fragment, (node, parents) => {
    const names = parents.filter((p) => p.type === "RegularElement" || p.type === "SvelteElement").map((p) => p.name);
    if (node.type === "Text") {
      if (names.some((n) => ["script", "style", "code", "pre", "kbd", "title"].includes(n))) return;
      if (isProse(node.data)) push("text", node.start, node.data);
    }
    if (node.type === "ExpressionTag") {
      for (const f of proseInExpression(node.expression)) push("text in an expression", f.start, f.text);
    }
    if (node.type === "Attribute" && READ_ATTRIBUTES.has(node.name)) {
      const parts = node.value === true ? [] : Array.isArray(node.value) ? node.value : [node.value];
      for (const part of parts) {
        if (part.type === "Text" && isProse(part.data)) push(`attribute ${node.name}`, part.start, part.data);
        if (part.type === "ExpressionTag") {
          for (const f of proseInExpression(part.expression)) push(`attribute ${node.name}`, f.start, f.text);
        }
      }
    }
  });
  for (const script of [ast.instance, ast.module]) {
    if (!script) continue;
    for (const f of proseInExpression(script.content)) push("script", f.start, f.text);
  }
  return out;
}

/** Findings in a .ts file. */
export function scanTs(source) {
  const file = ts.createSourceFile("x.ts", source, ts.ScriptTarget.ES2022, true);
  const out = [];
  const push = (node, text) => out.push({ line: file.getLineAndCharacterOfPosition(node.getStart()).line + 1, kind: "string", text: text.replace(/\s+/g, " ").trim() });
  const visit = (node) => {
    const parent = node.parent;
    const skip =
      !parent ||
      ts.isImportDeclaration(parent) ||
      ts.isExportDeclaration(parent) ||
      ts.isLiteralTypeNode(parent) ||
      ts.isPropertyAssignment(parent) && parent.name === node ||
      ts.isPropertySignature(parent) ||
      ts.isCaseClause(parent) ||
      ts.isEnumMember(parent) ||
      ts.isImportTypeNode(parent) ||
      ts.isModuleDeclaration(parent) ||
      (ts.isBinaryExpression(parent) && [ts.SyntaxKind.EqualsEqualsEqualsToken, ts.SyntaxKind.ExclamationEqualsEqualsToken, ts.SyntaxKind.EqualsEqualsToken, ts.SyntaxKind.ExclamationEqualsToken].includes(parent.operatorToken.kind)) ||
      (ts.isCallExpression(parent) && ts.isPropertyAccessExpression(parent.expression) && parent.expression.expression.getText() === "console");
    if ((ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) && !skip) {
      if (isProse(node.text)) push(node, node.text);
    } else if (ts.isTemplateExpression(node) && !skip) {
      const parts = [node.head.text, ...node.templateSpans.map((s) => s.literal.text)];
      if (parts.some((p) => isProse(p) || /\p{L}{3,}.*\s+\p{L}{2,}/u.test(p))) push(node, parts.join("{}"));
    }
    ts.forEachChild(node, visit);
  };
  visit(file);
  return out;
}
