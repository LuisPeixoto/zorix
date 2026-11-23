// Keeps the desktop app's text in the catalogue (ADR-0040). Run: `node tools/check-text.mjs`.
//
// Fails on any sentence written in a component or a helper instead of in `lib/i18n/messages.en.ts`: text between tags, the attributes a person
// reads (aria-label, title, placeholder, label, hint ...), the text of an event handler's toast, and string or template literals that read as
// prose in a script or a .ts file. See tools/lib/text-scan.mjs for what counts as prose.
//
// A real exception goes in ALLOW below with the reason beside it; an exception without a reason, or one that matches nothing, is itself an error.
import { readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { scanSvelte, scanTs } from "./lib/text-scan.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const src = join(root, "apps/desktop/src");

/**
 * `file` is relative to apps/desktop/src. Without `text` the whole file is exempt; with `text`, only the findings that contain it.
 * Keep this list short.
 */
const ALLOW = [
  { file: "lib/i18n/messages.en.ts", reason: "this is the catalogue: the sentences have to be somewhere" },
  { file: "lib/ui/icons.ts", reason: "SVG path data and icon names, not text" },
  { file: "lib/views/DesignSystem.svelte", reason: "the design system showcase (⌘⇧D) is a developer screen that documents the kit in English, not a screen of the product" },
];

const walk = (dir) =>
  readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return name === "node_modules" ? [] : walk(path);
    return /\.(svelte|ts)$/.test(name) && !/\.test\.ts$/.test(name) ? [path] : [];
  });

const problems = [];
const used = new Set();
for (const entry of ALLOW) if (entry.reason.trim().length < 10) problems.push(`The exception for ${entry.file} needs a reason of its own.`);

for (const path of walk(src)) {
  const file = relative(src, path).replace(/\\/g, "/");
  const exempt = ALLOW.filter((a) => a.file === file && !a.text);
  if (exempt.length) {
    exempt.forEach((a) => used.add(a));
    continue;
  }
  const source = readFileSync(path, "utf8");
  let found;
  try {
    found = file.endsWith(".svelte") ? scanSvelte(source) : scanTs(source);
  } catch (error) {
    problems.push(`${file}: could not be read: ${error.message}`);
    continue;
  }
  for (const f of found) {
    const allowed = ALLOW.find((a) => a.file === file && a.text && f.text.includes(a.text));
    if (allowed) {
      used.add(allowed);
      continue;
    }
    problems.push(`${file}:${f.line}: ${f.kind} “${f.text.slice(0, 80)}” is written in the code; put it in the catalogue (lib/i18n/messages.en.ts) and use t("key")`);
  }
}
for (const entry of ALLOW) if (!used.has(entry)) problems.push(`The exception for ${entry.file}${entry.text ? ` (“${entry.text}”)` : ""} matches nothing; remove it.`);

if (problems.length) {
  console.error(problems.join("\n"));
  console.error(`\n${problems.length} problem(s). See docs/adr/0040-interface-text-and-error-reasons.md.`);
  process.exit(1);
}
console.log(`text lint ok (${walk(src).length} files)`);
