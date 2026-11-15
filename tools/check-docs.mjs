// Checks the Markdown documentation: every relative link points to a file that exists, every #anchor points to a
// heading, and every image exists. Run from anywhere: `node tools/check-docs.mjs`.
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const skip = new Set(["node_modules", "target", ".git", "dist", "gen"]);

function markdownFiles(dir) {
  const found = [];
  for (const name of readdirSync(dir)) {
    if (skip.has(name)) continue;
    const path = join(dir, name);
    if (statSync(path).isDirectory()) found.push(...markdownFiles(path));
    else if (name.endsWith(".md")) found.push(path);
  }
  return found;
}

// GitHub's rule for heading ids: lowercase, drop punctuation, spaces become dashes.
const slug = (heading) =>
  heading
    .trim()
    .toLowerCase()
    .replace(/[`*_~]/g, "")
    .replace(/[^\p{L}\p{N}\s-]/gu, "")
    .replace(/\s/g, "-");

const anchorsCache = new Map();
function anchors(file) {
  if (!anchorsCache.has(file)) {
    const ids = new Set();
    let inFence = false;
    for (const line of readFileSync(file, "utf8").split("\n")) {
      if (line.startsWith("```")) inFence = !inFence;
      const heading = !inFence && line.match(/^#{1,6}\s+(.*)$/);
      if (heading) ids.add(slug(heading[1]));
    }
    anchorsCache.set(file, ids);
  }
  return anchorsCache.get(file);
}

let problems = 0;
const report = (file, line, message) => {
  console.error(`${relative(root, file)}:${line}: ${message}`);
  problems += 1;
};

for (const file of markdownFiles(root)) {
  let inFence = false;
  readFileSync(file, "utf8")
    .split("\n")
    .forEach((text, index) => {
      if (text.startsWith("```")) inFence = !inFence;
      if (inFence) return;
      const withoutCode = text.replace(/`[^`]*`/g, "");
      for (const match of withoutCode.matchAll(/!?\[[^\]]*\]\(([^)\s]+)\)|src="([^"]+)"/g)) {
        const target = match[1] ?? match[2];
        if (/^(https?:|mailto:)/.test(target)) continue;
        const [pathPart, anchor] = target.split("#");
        const destination = pathPart ? resolve(dirname(file), pathPart) : file;
        if (!existsSync(destination)) {
          report(file, index + 1, `broken link: ${target}`);
        } else if (anchor && destination.endsWith(".md") && !anchors(destination).has(anchor)) {
          report(file, index + 1, `no heading “${anchor}” in ${relative(root, destination)}`);
        }
      }
    });
}

if (problems > 0) {
  console.error(`\n${problems} documentation problem(s).`);
  process.exit(1);
}
console.log(`documentation links ok (${markdownFiles(root).length} files)`);
