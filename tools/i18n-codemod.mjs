// One-off migration (ADR-0040): moves the interface's literal text into the catalogue and replaces it with `t("key")` calls. Run it from the
// repository root: `node tools/i18n-codemod.mjs [--write]`. Without --write it only reports. The result is checked by the type check, the
// tests (English must stay identical), the string lint and a look at the screens; whatever it cannot do safely it lists for hand work.
import { createRequire } from "node:module";
import { readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { READ_ATTRIBUTES, isProse } from "./lib/text-scan.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const src = join(root, "apps/desktop/src");
const require = createRequire(join(root, "apps/desktop/package.json"));
const { parse } = require("svelte/compiler");
const ts = require("typescript");
const write = process.argv.includes("--write");

const SKIP_FILES = new Set(["lib/views/DesignSystem.svelte", "lib/ui/icons.ts", "lib/i18n/messages.en.ts", "lib/i18n/core.ts", "lib/i18n/index.ts", "lib/i18n/format.ts"]);
const walk = (dir) =>
  readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return name === "node_modules" ? [] : walk(path);
    return /\.(svelte|ts)$/.test(name) && !/\.test\.ts$/.test(name) ? [path] : [];
  });

// ---------------------------------------------------------------------------------------------------------------------------------
// Keys

const camel = (words) => words.map((w, i) => (i === 0 ? w.toLowerCase() : w[0].toUpperCase() + w.slice(1).toLowerCase())).join("");
function scopeOf(file) {
  let rel = file.replace(/\.svelte\.ts$|\.svelte$|\.ts$/, "").replace(/^lib\//, "").replace(/^components\//, "");
  const parts = rel.split("/").map((p) => p.replace(/[^A-Za-z0-9]+/g, ""));
  return parts.map((p, i) => p[0].toLowerCase() + p.slice(1)).join(".");
}
function slugOf(message) {
  const words = message.replace(/\{[^}]*\}/g, " ").match(/[\p{L}\p{N}]+/gu) ?? [];
  const ascii = words.map((w) => w.normalize("NFD").replace(/[̀-ͯ]/g, "")).filter(Boolean);
  return camel(ascii.slice(0, 4)) || "text";
}

// The catalogue being built: key -> message, and (scope + message) -> key, so one sentence in one place has one key.
const catalogue = new Map();
const keyOf = new Map();
const scopesOfMessage = new Map(); // message JSON -> Set of scopes
function claim(scope, message) {
  const id = JSON.stringify(message);
  (scopesOfMessage.get(id) ?? scopesOfMessage.set(id, new Set()).get(id)).add(scope);
  const known = keyOf.get(`${scope}\u0000${id}`);
  if (known) return known;
  const slug = typeof message === "string" ? slugOf(message) : slugOf(message.other);
  let key = `${scope}.${slug}`;
  let n = 2;
  while (catalogue.has(key)) key = `${scope}.${slug}${n++}`;
  catalogue.set(key, message);
  keyOf.set(`${scope}\u0000${id}`, key);
  return key;
}

// ---------------------------------------------------------------------------------------------------------------------------------
// Messages from the pieces of a run of text and expressions

const report = { manual: [], files: 0, replaced: 0 };
const note = (file, line, why, text) => report.manual.push(`${file}:${line}: ${why}: ${String(text).replace(/\s+/g, " ").slice(0, 90)}`);

/** A name for a placeholder from the expression's source: `rows.length` -> `length`, `name` -> `name`, anything else -> `v1`. */
function placeholderName(source, used) {
  const m = source.trim().match(/([A-Za-z_][A-Za-z0-9_]*)\s*(?:\(\s*\))?$/);
  let name = m && !/^(true|false|null|undefined)$/.test(m[1]) ? m[1] : "value";
  if (name === "count") name = "total";
  let candidate = name;
  let n = 2;
  while (used.has(candidate)) candidate = `${name}${n++}`;
  return candidate;
}

/**
 * Parts are `{ text }` or `{ expr: "source", node }`. Returns { message, args } where a plural ternary becomes forms, or null when it
 * cannot be done safely.
 */
function buildMessage(parts) {
  const isPluralTernary = (node) => {
    if (!node || node.type !== "ConditionalExpression") return null;
    const { test, consequent, alternate } = node;
    const lit = (n) => n?.type === "Literal" && typeof n.value === "string";
    if (!lit(consequent) || !lit(alternate)) return null;
    if (test.type === "BinaryExpression" && ["===", "=="].includes(test.operator) && test.right.type === "Literal" && test.right.value === 1)
      return { count: test.left, one: consequent.value, other: alternate.value };
    if (test.type === "BinaryExpression" && ["!==", "!="].includes(test.operator) && test.right.type === "Literal" && test.right.value === 1)
      return { count: test.left, one: alternate.value, other: consequent.value };
    if (test.type === "BinaryExpression" && test.operator === ">" && test.right.type === "Literal" && test.right.value === 1)
      return { count: test.left, one: alternate.value, other: consequent.value };
    return null;
  };
  const plural = parts.map((p) => (p.node ? isPluralTernary(p.node) : null));
  const countSources = [...new Set(plural.filter(Boolean).map((p) => p.countSource))];
  const withCount = parts.map((p, i) => (plural[i] ? { ...p, plural: plural[i], countSource: p.sourceOf(plural[i].count) } : p));
  const counts = [...new Set(withCount.filter((p) => p.plural).map((p) => p.countSource))];
  if (counts.length > 1) return null;
  const used = new Set();
  const args = {};
  const names = new Map(); // expression source -> placeholder
  const place = (source) => {
    if (counts.length === 1 && source === counts[0]) {
      args.count = source;
      used.add("count");
      return "count";
    }
    if (names.has(source)) return names.get(source);
    const name = placeholderName(source, used);
    used.add(name);
    names.set(source, name);
    args[name] = source;
    return name;
  };
  const forms = counts.length === 1 ? ["one", "other"] : ["other"];
  const out = {};
  for (const form of forms) {
    let text = "";
    for (const p of withCount) {
      if (p.text !== undefined) text += p.text;
      else if (p.plural) text += p.plural[form];
      else text += `{${place(p.expr)}}`;
    }
    out[form] = text.replace(/[ \t]*\n[ \t]*/g, " ").replace(/ {2,}/g, " ");
  }
  const message = forms.length === 1 ? out.other : out;
  return { message, args };
}

const argsSource = (args) => {
  const entries = Object.entries(args);
  if (!entries.length) return "";
  return `, { ${entries.map(([k, v]) => (k === v ? k : `${k}: ${v}`)).join(", ")} }`;
};

// ---------------------------------------------------------------------------------------------------------------------------------
// JavaScript: prose literals inside an expression or a program (ESTree from svelte, or TypeScript's own tree)

/** Edits for the prose string and template literals of an ESTree subtree. Returns [{ start, end, replace }]. */
function estreeLiteralEdits(tree, source, scope, call, file) {
  const edits = [];
  const visit = (node, parents) => {
    if (!node || typeof node !== "object") return;
    if (Array.isArray(node)) return node.forEach((c) => visit(c, parents));
    if (typeof node.type !== "string") return;
    const parent = parents[parents.length - 1];
    const typed = parents.some((p) => p.type.startsWith("TS")) || node.type.startsWith("TS");
    if (typed) return;
    const skipParent =
      parent &&
      (parent.type === "ImportDeclaration" ||
        parent.type === "ExportNamedDeclaration" ||
        parent.type === "ExportAllDeclaration" ||
        (parent.type === "Property" && parent.key === node && !parent.computed) ||
        (parent.type === "BinaryExpression" && ["===", "!==", "==", "!=", "in"].includes(parent.operator)) ||
        parent.type === "SwitchCase" ||
        (parent.type === "CallExpression" && parent.callee.type === "MemberExpression" && parent.callee.object.name === "console"));
    if (node.type === "Literal" && typeof node.value === "string" && !skipParent && isProse(node.value)) {
      edits.push({ start: node.start, end: node.end, replace: `${call}(${JSON.stringify(claim(scope, node.value))})` });
      return;
    }
    if (node.type === "TemplateLiteral" && !skipParent) {
      const hasProse = node.quasis.some((q) => isProse(q.value.cooked ?? q.value.raw));
      if (hasProse) {
        const parts = [];
        node.quasis.forEach((q, i) => {
          parts.push({ text: q.value.cooked ?? q.value.raw });
          if (node.expressions[i]) {
            const e = node.expressions[i];
            const text = transformedSource(e, source, scope, call, file);
            parts.push({ expr: text, node: e, sourceOf: (n) => transformedSource(n, source, scope, call, file) });
          }
        });
        const built = buildMessage(parts);
        if (built) {
          edits.push({ start: node.start, end: node.end, replace: `${call}(${JSON.stringify(claim(scope, built.message))}${argsSource(built.args)})` });
          return;
        }
        note(file, lineAt(source, node.start), "a template with more than one count", source.slice(node.start, node.end));
      }
    }
    for (const key of Object.keys(node)) {
      if (key === "parent" || key === "metadata" || key === "loc") continue;
      visit(node[key], [...parents, node]);
    }
  };
  visit(tree, []);
  return edits;
}

/** The source text of a node with its own literals already replaced. */
function transformedSource(node, source, scope, call, file) {
  const inner = estreeLiteralEdits(node, source, scope, call, file).filter((e) => e.start >= node.start && e.end <= node.end);
  return applyEdits(source.slice(node.start, node.end), inner.map((e) => ({ ...e, start: e.start - node.start, end: e.end - node.start })));
}

function applyEdits(text, edits) {
  const sorted = [...edits].sort((a, b) => b.start - a.start);
  let out = text;
  let last = Infinity;
  for (const e of sorted) {
    if (e.end > last) continue; // nested inside a larger edit that was already applied
    out = out.slice(0, e.start) + e.replace + out.slice(e.end);
    last = e.start;
  }
  return out;
}

const lineAt = (source, index) => source.slice(0, index).split("\n").length;

// ---------------------------------------------------------------------------------------------------------------------------------
// A .svelte file

function svelteEdits(source, file, scope, call) {
  const ast = parse(source, { modern: true });
  const edits = [];
  const skipNames = new Set(["script", "style", "code", "pre", "kbd", "title"]);

  const visitFragment = (fragment, names) => {
    const nodes = fragment.nodes;
    let i = 0;
    while (i < nodes.length) {
      const first = nodes[i];
      if (first.type !== "Text" && first.type !== "ExpressionTag") {
        visitNode(first, names);
        i++;
        continue;
      }
      // A run of text and expressions that read as one sentence.
      let j = i;
      while (j < nodes.length && (nodes[j].type === "Text" || nodes[j].type === "ExpressionTag")) j++;
      const run = nodes.slice(i, j);
      i = j;
      const insideSkipped = names.some((n) => skipNames.has(n));
      const hasProse = run.some((n) => n.type === "Text" && isProse(n.data));
      // An expression with prose literals of its own (a ternary of two sentences) is handled one literal at a time.
      const proseExpr = run.some((n) => n.type === "ExpressionTag" && estreeLiteralEdits(n.expression, source, scope, call, file).length > 0);
      if (insideSkipped) continue;
      if (!hasProse) {
        for (const n of run) if (n.type === "ExpressionTag") edits.push(...estreeLiteralEdits(n.expression, source, scope, call, file));
        continue;
      }
      const hasLiteralExpr = run.some((n) => n.type === "ExpressionTag" && estreeLiteralEdits(n.expression, source, scope, call, file).length > 0 && !isPluralOnly(n));
      // Trim the whitespace at both ends of the run: it stays where it is, outside the replacement.
      const startNode = run[0];
      const endNode = run[run.length - 1];
      const lead = startNode.type === "Text" ? startNode.data.match(/^\s*/)[0].length : 0;
      const trail = endNode.type === "Text" ? endNode.data.match(/\s*$/)[0].length : 0;
      const rawStart = startNode.start + (startNode.type === "Text" ? sourceLead(source, startNode) : 0);
      const rawEnd = endNode.end - (endNode.type === "Text" ? sourceTrail(source, endNode) : 0);
      void lead;
      void trail;
      if (rawEnd <= rawStart) continue;
      const parts = run.map((n, idx) => {
        if (n.type === "Text") {
          let text = n.data;
          if (idx === 0) text = text.replace(/^\s+/, "");
          if (idx === run.length - 1) text = text.replace(/\s+$/, "");
          return { text };
        }
        const expr = transformedSource(n.expression, source, scope, call, file);
        return { expr, node: n.expression, sourceOf: (x) => transformedSource(x, source, scope, call, file) };
      });
      if (hasLiteralExpr && !proseOnlyPlural(run)) {
        // A sentence with an expression that holds sentences too: do each piece by itself.
        for (const n of run) {
          if (n.type === "ExpressionTag") edits.push(...estreeLiteralEdits(n.expression, source, scope, call, file));
          else if (isProse(n.data)) {
            const core = coreOf(source, n);
            if (core) edits.push({ start: core.start, end: core.end, replace: `{${call}(${JSON.stringify(claim(scope, n.data.trim().replace(/\s+/g, " ")))})}` });
          }
        }
        continue;
      }
      void proseExpr;
      const built = buildMessage(parts);
      if (!built) {
        note(file, lineAt(source, startNode.start), "text with more than one count", source.slice(rawStart, rawEnd));
        continue;
      }
      edits.push({ start: rawStart, end: rawEnd, replace: `{${call}(${JSON.stringify(claim(scope, built.message))}${argsSource(built.args)})}` });
    }
  };

  const isPluralOnly = () => false;
  const proseOnlyPlural = (run) => run.every((n) => n.type === "Text" || isPluralNode(n.expression) || !hasProseLiteral(n.expression));
  const isPluralNode = (e) => {
    if (e.type !== "ConditionalExpression") return false;
    const t = e.test;
    return t.type === "BinaryExpression" && t.right?.type === "Literal" && t.right.value === 1 && e.consequent.type === "Literal" && e.alternate.type === "Literal";
  };
  const hasProseLiteral = (e) => estreeLiteralEdits(e, source, scope, call, file).length > 0;

  function sourceLead(source, node) {
    return source.slice(node.start, node.end).match(/^\s*/)[0].length;
  }
  function sourceTrail(source, node) {
    return source.slice(node.start, node.end).match(/\s*$/)[0].length;
  }
  function coreOf(source, node) {
    const raw = source.slice(node.start, node.end);
    const lead = raw.match(/^\s*/)[0].length;
    const trail = raw.match(/\s*$/)[0].length;
    return raw.trim() ? { start: node.start + lead, end: node.end - trail } : null;
  }

  const visitAttributes = (node) => {
    for (const attr of node.attributes ?? []) {
      if (attr.type !== "Attribute" || !READ_ATTRIBUTES.has(attr.name) || attr.value === true) continue;
      const parts = Array.isArray(attr.value) ? attr.value : [attr.value];
      if (parts.length === 1 && parts[0].type === "ExpressionTag") {
        edits.push(...estreeLiteralEdits(parts[0].expression, source, scope, call, file));
        continue;
      }
      const proseText = parts.some((p) => p.type === "Text" && isProse(p.data));
      if (!proseText) continue;
      const built = buildMessage(
        parts.map((p) =>
          p.type === "Text"
            ? { text: p.data }
            : { expr: transformedSource(p.expression, source, scope, call, file), node: p.expression, sourceOf: (x) => transformedSource(x, source, scope, call, file) },
        ),
      );
      if (!built) {
        note(file, lineAt(source, attr.start), "an attribute with more than one count", source.slice(attr.start, attr.end));
        continue;
      }
      edits.push({ start: attr.start, end: attr.end, replace: `${attr.name}={${call}(${JSON.stringify(claim(scope, built.message))}${argsSource(built.args)})}` });
    }
  };

  function visitNode(node, names) {
    if (!node || typeof node !== "object") return;
    const here = node.type === "RegularElement" || node.type === "SvelteElement" ? [...names, node.name] : names;
    if (node.type === "RegularElement" || node.type === "Component" || node.type === "SvelteElement" || node.type === "SvelteComponent" || node.type === "SvelteFragment") visitAttributes(node);
    // Expressions of blocks and tags other than text (`{#if a ? "x" : "y"}` is not text, but `{@render x("Label")}` can carry some).
    if (node.type === "RenderTag" || node.type === "ConstTag" || node.type === "HtmlTag") edits.push(...estreeLiteralEdits(node.expression ?? node.declaration, source, scope, call, file));
    for (const key of Object.keys(node)) {
      if (["type", "start", "end", "attributes", "expression", "parent", "metadata", "name"].includes(key) && key !== "expression") continue;
      const value = node[key];
      if (!value || typeof value !== "object") continue;
      if (value.type === "Fragment") visitFragment(value, here);
      else if (Array.isArray(value)) value.forEach((v) => v && typeof v === "object" && (v.type === "Fragment" ? visitFragment(v, here) : visitNode(v, here)));
      else if (typeof value.type === "string" && key !== "expression") visitNode(value, here);
    }
  }

  visitFragment(ast.fragment, []);
  for (const script of [ast.instance, ast.module]) if (script) edits.push(...estreeLiteralEdits(script.content, source, scope, call, file));
  return { edits, ast };
}

// ---------------------------------------------------------------------------------------------------------------------------------
// A .ts file (TypeScript's own tree, since svelte's parser is for components)

function tsEdits(source, file, scope, call) {
  const sf = ts.createSourceFile("x.ts", source, ts.ScriptTarget.ES2022, true);
  const edits = [];
  const exprSource = (node) => transformTs(node);
  const transformTs = (node) => {
    const inner = [];
    collect(node, inner);
    const base = node.getStart();
    return applyEdits(source.slice(base, node.end), inner.map((e) => ({ ...e, start: e.start - base, end: e.end - base })));
  };
  const skipParent = (node) => {
    const parent = node.parent;
    if (!parent) return true;
    return (
      ts.isImportDeclaration(parent) ||
      ts.isExportDeclaration(parent) ||
      ts.isLiteralTypeNode(parent) ||
      (ts.isPropertyAssignment(parent) && parent.name === node) ||
      ts.isPropertySignature(parent) ||
      ts.isCaseClause(parent) ||
      ts.isEnumMember(parent) ||
      ts.isImportTypeNode(parent) ||
      ts.isModuleDeclaration(parent) ||
      (ts.isBinaryExpression(parent) && [ts.SyntaxKind.EqualsEqualsEqualsToken, ts.SyntaxKind.ExclamationEqualsEqualsToken, ts.SyntaxKind.EqualsEqualsToken, ts.SyntaxKind.ExclamationEqualsToken].includes(parent.operatorToken.kind)) ||
      (ts.isCallExpression(parent) && ts.isPropertyAccessExpression(parent.expression) && parent.expression.expression.getText() === "console") ||
      inTypePosition(node)
    );
  };
  const inTypePosition = (node) => {
    for (let p = node.parent; p; p = p.parent) if (ts.isTypeNode(p) || ts.isTypeAliasDeclaration(p) || ts.isInterfaceDeclaration(p)) return true;
    return false;
  };
  const pluralOf = (e) => {
    if (!ts.isConditionalExpression(e)) return null;
    const c = e.condition;
    const str = (n) => ts.isStringLiteral(n) || ts.isNoSubstitutionTemplateLiteral(n);
    if (!str(e.whenTrue) || !str(e.whenFalse) || !ts.isBinaryExpression(c) || !ts.isNumericLiteral(c.right) || c.right.text !== "1") return null;
    const op = c.operatorToken.kind;
    if (op === ts.SyntaxKind.EqualsEqualsEqualsToken || op === ts.SyntaxKind.EqualsEqualsToken) return { count: c.left, one: e.whenTrue.text, other: e.whenFalse.text };
    if (op === ts.SyntaxKind.ExclamationEqualsEqualsToken || op === ts.SyntaxKind.ExclamationEqualsToken) return { count: c.left, one: e.whenFalse.text, other: e.whenTrue.text };
    if (op === ts.SyntaxKind.GreaterThanToken) return { count: c.left, one: e.whenFalse.text, other: e.whenTrue.text };
    return null;
  };
  const collect = (root, into) => {
    const visit = (node) => {
      if ((ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) && !skipParent(node) && isProse(node.text)) {
        into.push({ start: node.getStart(), end: node.end, replace: `${call}(${JSON.stringify(claim(scope, node.text))})` });
        return;
      }
      if (ts.isTemplateExpression(node) && !skipParent(node)) {
        const texts = [node.head.text, ...node.templateSpans.map((s) => s.literal.text)];
        if (texts.some((p) => isProse(p) || /\p{L}{3,}.*\s+\p{L}{2,}/u.test(p))) {
          const parts = [{ text: node.head.text }];
          for (const span of node.templateSpans) {
            const plural = pluralOf(span.expression);
            const countSource = plural ? exprSource(plural.count) : null;
            parts.push({ expr: exprSource(span.expression), node: plural ? { type: "ConditionalExpression" } : null, tsPlural: plural, countSource, sourceOf: () => countSource });
            parts.push({ text: span.literal.text });
          }
          const built = buildTsMessage(parts);
          if (built) {
            into.push({ start: node.getStart(), end: node.end, replace: `${call}(${JSON.stringify(claim(scope, built.message))}${argsSource(built.args)})` });
            return;
          }
          note(file, sf.getLineAndCharacterOfPosition(node.getStart()).line + 1, "a template with more than one count", node.getText());
        }
      }
      ts.forEachChild(node, visit);
    };
    visit(root);
  };
  const buildTsMessage = (parts) => {
    const counts = [...new Set(parts.filter((p) => p.tsPlural).map((p) => p.countSource))];
    if (counts.length > 1) return null;
    const used = new Set();
    const args = {};
    const names = new Map();
    const place = (source) => {
      if (counts.length === 1 && source === counts[0]) {
        args.count = source;
        used.add("count");
        return "count";
      }
      if (names.has(source)) return names.get(source);
      const name = placeholderName(source, used);
      used.add(name);
      names.set(source, name);
      args[name] = source;
      return name;
    };
    const forms = counts.length === 1 ? ["one", "other"] : ["other"];
    const out = {};
    for (const form of forms) {
      let text = "";
      for (const p of parts) {
        if (p.text !== undefined) text += p.text;
        else if (p.tsPlural) text += p.tsPlural[form];
        else text += `{${place(p.expr)}}`;
      }
      out[form] = text;
    }
    return { message: forms.length === 1 ? out.other : out, args };
  };
  collect(sf, edits);
  return { edits, sf };
}

// ---------------------------------------------------------------------------------------------------------------------------------
// Imports

function usesIdentifier(source, name) {
  // A rough check that `name` is already something in this file (a variable, a parameter, a loop variable).
  const re = new RegExp(`(?<![\\w$."'\`])${name}(?![\\w$"'\`])`);
  return source.split("\n").some((line) => re.test(line.replace(/(["'`])(?:\\.|(?!\1).)*\1/g, '""')));
}

function importFor(file, call) {
  const from = relative(dirname(join(src, file)), join(src, "lib/i18n/index.ts")).replace(/\\/g, "/");
  const path = from.startsWith(".") ? from : `./${from}`;
  return call === "t" ? `import { t } from "${path}";` : `import { t as ${call} } from "${path}";`;
}

// ---------------------------------------------------------------------------------------------------------------------------------
// Run

const files = walk(src)
  .map((p) => relative(src, p))
  .filter((f) => !SKIP_FILES.has(f))
  .sort();

const plans = [];
for (const file of files) {
  const source = readFileSync(join(src, file), "utf8");
  const scope = scopeOf(file);
  const call = usesIdentifier(source, "t") ? "tr" : "t";
  let result;
  try {
    result = file.endsWith(".svelte") ? svelteEdits(source, file, scope, call) : tsEdits(source, file, scope, call);
  } catch (error) {
    note(file, 0, "could not be read", error.message);
    continue;
  }
  if (result.edits.length) plans.push({ file, source, call, edits: result.edits, ast: result.ast });
}

// Sentences that three or more parts of the app say the same way live under `common.`.
const promoted = new Map();
for (const [id, scopes] of scopesOfMessage) {
  if (scopes.size >= 3) {
    const message = JSON.parse(id);
    const slug = slugOf(typeof message === "string" ? message : message.other);
    let key = `common.${slug}`;
    let n = 2;
    while (catalogue.has(key) && JSON.stringify(catalogue.get(key)) !== id) key = `common.${slug}${n++}`;
    promoted.set(id, key);
  }
}
const finalKey = (key) => {
  const id = JSON.stringify(catalogue.get(key));
  return promoted.get(id) ?? key;
};

for (const plan of plans) {
  const edits = plan.edits.map((e) => ({ ...e, replace: e.replace.replace(/\bt(r?)\("([^"]+)"/, (m, r, key) => `${r ? "tr" : "t"}(${JSON.stringify(finalKey(key))}`) }));
  // The call name is the file's; the pattern above only rewrites the first call of each edit, so rewrite them all.
  const rewritten = plan.edits.map((e) => ({ ...e, replace: e.replace.replace(/\b(tr?)\("([^"]+)"/g, (m, name, key) => `${name}(${JSON.stringify(finalKey(key))}`) }));
  void edits;
  let out = applyEdits(plan.source, rewritten);
  const importLine = importFor(plan.file, plan.call);
  if (plan.file.endsWith(".svelte")) {
    const instance = plan.ast.instance;
    if (instance) {
      const open = plan.source.indexOf(">", instance.start) + 1;
      const delta = out.length - plan.source.length;
      void delta;
      // Positions of the script's start did not move (edits are after it in the file, or inside it after the opening tag).
      out = out.slice(0, open) + `\n  ${importLine}` + out.slice(open);
    } else {
      out = `<script lang="ts">\n  ${importLine}\n</script>\n\n${out}`;
    }
  } else {
    const imports = [...out.matchAll(/^import .*;$/gm)];
    if (imports.length) {
      const last = imports[imports.length - 1];
      const at = last.index + last[0].length;
      out = out.slice(0, at) + `\n${importLine}` + out.slice(at);
    } else {
      out = `${importLine}\n${out}`;
    }
  }
  report.files++;
  report.replaced += plan.edits.length;
  if (write) writeFileSync(join(src, plan.file), out);
}

// The catalogue, grouped by scope in the order the keys were made.
const finalCatalogue = new Map();
for (const [key, message] of catalogue) {
  const fk = finalKey(key);
  if (!finalCatalogue.has(fk)) finalCatalogue.set(fk, message);
}
const sorted = [...finalCatalogue.entries()].sort(([a], [b]) => a.localeCompare(b));
const lines = sorted.map(([key, message]) => {
  const value =
    typeof message === "string"
      ? JSON.stringify(message)
      : `{ ${Object.entries(message).map(([form, text]) => `${form}: ${JSON.stringify(text)}`).join(", ")} }`;
  return `  ${JSON.stringify(key)}: ${value},`;
});
const catalogueSource = `// The source catalogue of the interface (ADR-0040): English. A message is a sentence with \`{placeholders}\`, or its plural forms
// (\`one\`, \`other\`) for a message that counts something. Keys are grouped by the part of the app that shows them.
import type { Message } from "./core.ts";

export const en = {
${lines.join("\n")}
} as const satisfies Record<string, Message>;
`;
if (write) writeFileSync(join(src, "lib/i18n/messages.en.ts"), catalogueSource);

console.log(`${report.files} files, ${report.replaced} replacements, ${finalCatalogue.size} messages (${promoted.size} promoted to common)`);
console.log(`${report.manual.length} places left for hand work:`);
console.log(report.manual.join("\n"));
